//! Postgres introspection (milestone M1).
//!
//! The single place that knows how to read `pg_catalog`. Produces a
//! [`Catalog`] snapshot — tables, columns with their SQL types and
//! nullability, primary keys and foreign keys — which the planner combines
//! with metadata. Nothing downstream talks to `pg_catalog` directly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// Keyed by "schema.table".
    pub tables: BTreeMap<String, TableInfo>,
    /// Keyed by "schema.function". Overloads are not supported.
    pub functions: BTreeMap<String, FunctionInfo>,
}

impl Catalog {
    pub fn table(&self, schema: &str, name: &str) -> Option<&TableInfo> {
        self.tables.get(&format!("{schema}.{name}"))
    }

    pub fn function(&self, schema: &str, name: &str) -> Option<&FunctionInfo> {
        self.functions.get(&format!("{schema}.{name}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub schema: String,
    pub name: String,
    pub args: Vec<FunctionArg>,
    /// If the function returns (setof) a known table's row type.
    pub returns_table: Option<(String, String)>,
    pub returns_set: bool,
    /// Scalar return type name when not returning a table row.
    pub returns_scalar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionArg {
    pub name: Option<String>,
    /// The argument has a DEFAULT and may be omitted from calls.
    #[serde(default)]
    pub has_default: bool,
    pub pg_type: String,
    /// Set when the argument type is the row type of a known table.
    pub composite_of: Option<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
}

impl TableInfo {
    pub fn column(&self, name: &str) -> Option<&ColumnInfo> {
        self.columns.iter().find(|c| c.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    /// Postgres type name as reported by pg_catalog (e.g. `int4`, `text`).
    pub pg_type: String,
    pub nullable: bool,
    pub has_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey {
    pub constraint_name: String,
    /// Local column -> referenced column.
    pub column_mapping: BTreeMap<String, String>,
    pub referenced_schema: String,
    pub referenced_table: String,
}

const COLUMNS_SQL: &str = r#"
SELECT n.nspname, c.relname, a.attname, t.typname,
       NOT a.attnotnull AS nullable,
       a.atthasdef AS has_default
FROM pg_attribute a
JOIN pg_class c ON a.attrelid = c.oid
JOIN pg_namespace n ON c.relnamespace = n.oid
JOIN pg_type t ON a.atttypid = t.oid
WHERE c.relkind IN ('r', 'v', 'm', 'f', 'p')
  AND a.attnum > 0
  AND NOT a.attisdropped
  AND n.nspname NOT IN ('pg_catalog', 'information_schema', 'hdb_catalog')
  AND n.nspname NOT LIKE 'pg_toast%'
  AND n.nspname NOT LIKE 'pg_temp%'
ORDER BY n.nspname, c.relname, a.attnum
"#;

const PRIMARY_KEYS_SQL: &str = r#"
SELECT n.nspname, c.relname, a.attname
FROM pg_constraint con
JOIN pg_class c ON con.conrelid = c.oid
JOIN pg_namespace n ON c.relnamespace = n.oid
CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum
WHERE con.contype = 'p'
ORDER BY n.nspname, c.relname, k.ord
"#;

const FOREIGN_KEYS_SQL: &str = r#"
SELECT con.conname, n.nspname, c.relname,
       fn.nspname AS fschema, fc.relname AS ftable,
       a.attname AS col, fa.attname AS fcol
FROM pg_constraint con
JOIN pg_class c ON con.conrelid = c.oid
JOIN pg_namespace n ON c.relnamespace = n.oid
JOIN pg_class fc ON con.confrelid = fc.oid
JOIN pg_namespace fn ON fc.relnamespace = fn.oid
CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS fk(attnum, ord)
  ON fk.ord = k.ord
JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum
JOIN pg_attribute fa ON fa.attrelid = fc.oid AND fa.attnum = fk.attnum
WHERE con.contype = 'f'
ORDER BY con.conname, k.ord
"#;

const FUNCTIONS_SQL: &str = r#"
SELECT n.nspname,
       p.proname,
       p.proretset,
       p.pronargs::int4,
       p.pronargdefaults::int4,
       rt.typname AS ret_type,
       rn.nspname AS ret_rel_schema,
       rc.relname AS ret_rel_name,
       coalesce(p.proargnames, '{}'::text[]) AS arg_names,
       (SELECT coalesce(array_agg(at.typname ORDER BY a.ord), '{}'::name[])
        FROM unnest(p.proargtypes) WITH ORDINALITY AS a(oid, ord)
        JOIN pg_type at ON at.oid = a.oid) AS arg_types,
       (SELECT coalesce(array_agg(coalesce(an.nspname || '.' || ac.relname, '') ORDER BY a.ord), '{}'::text[])
        FROM unnest(p.proargtypes) WITH ORDINALITY AS a(oid, ord)
        JOIN pg_type at ON at.oid = a.oid
        LEFT JOIN pg_class ac ON ac.oid = at.typrelid AND ac.relkind IN ('r', 'v', 'm', 'p')
        LEFT JOIN pg_namespace an ON ac.relnamespace = an.oid) AS arg_composites
FROM pg_proc p
JOIN pg_namespace n ON p.pronamespace = n.oid
JOIN pg_type rt ON p.prorettype = rt.oid
LEFT JOIN pg_class rc ON rc.oid = rt.typrelid AND rc.relkind IN ('r', 'v', 'm', 'p')
LEFT JOIN pg_namespace rn ON rc.relnamespace = rn.oid
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema', 'hdb_catalog')
  AND n.nspname NOT LIKE 'pg_toast%'
  AND n.nspname NOT LIKE 'pg_temp%'
  AND p.prokind = 'f'
"#;

/// Take a full snapshot of user-visible relations.
pub async fn introspect(client: &Client) -> Result<Catalog, tokio_postgres::Error> {
    let mut catalog = Catalog::default();

    for row in client.query(COLUMNS_SQL, &[]).await? {
        let schema: String = row.get(0);
        let table: String = row.get(1);
        let key = format!("{schema}.{table}");
        let entry = catalog.tables.entry(key).or_insert_with(|| TableInfo {
            schema,
            name: table,
            columns: vec![],
            primary_key: vec![],
            foreign_keys: vec![],
        });
        entry.columns.push(ColumnInfo {
            name: row.get(2),
            pg_type: row.get(3),
            nullable: row.get(4),
            has_default: row.get(5),
        });
    }

    for row in client.query(PRIMARY_KEYS_SQL, &[]).await? {
        let schema: String = row.get(0);
        let table: String = row.get(1);
        if let Some(info) = catalog.tables.get_mut(&format!("{schema}.{table}")) {
            info.primary_key.push(row.get(2));
        }
    }

    for row in client.query(FOREIGN_KEYS_SQL, &[]).await? {
        let conname: String = row.get(0);
        let schema: String = row.get(1);
        let table: String = row.get(2);
        let Some(info) = catalog.tables.get_mut(&format!("{schema}.{table}")) else {
            continue;
        };
        let fk = match info
            .foreign_keys
            .iter_mut()
            .find(|fk| fk.constraint_name == conname)
        {
            Some(fk) => fk,
            None => {
                info.foreign_keys.push(ForeignKey {
                    constraint_name: conname,
                    column_mapping: BTreeMap::new(),
                    referenced_schema: row.get(3),
                    referenced_table: row.get(4),
                });
                info.foreign_keys.last_mut().unwrap()
            }
        };
        fk.column_mapping.insert(row.get(5), row.get(6));
    }

    for row in client.query(FUNCTIONS_SQL, &[]).await? {
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let returns_set: bool = row.get(2);
        let nargs: i32 = row.get(3);
        let ndefaults: i32 = row.get(4);
        let ret_type: String = row.get(5);
        let ret_rel_schema: Option<String> = row.get(6);
        let ret_rel_name: Option<String> = row.get(7);
        let arg_names: Vec<String> = row.get(8);
        let arg_types: Vec<String> = row.get(9);
        let arg_composites: Vec<String> = row.get(10);
        let first_default = (nargs - ndefaults).max(0) as usize;

        let returns_table = ret_rel_schema.zip(ret_rel_name);
        let args = arg_types
            .iter()
            .enumerate()
            .map(|(i, pg_type)| FunctionArg {
                name: arg_names.get(i).filter(|n| !n.is_empty()).cloned(),
                has_default: i >= first_default,
                pg_type: pg_type.clone(),
                composite_of: arg_composites
                    .get(i)
                    .filter(|c| !c.is_empty())
                    .and_then(|c| {
                        c.split_once('.')
                            .map(|(s, t)| (s.to_string(), t.to_string()))
                    }),
            })
            .collect();

        catalog.functions.insert(
            format!("{schema}.{name}"),
            FunctionInfo {
                schema,
                name,
                args,
                returns_scalar: if returns_table.is_none() {
                    Some(ret_type)
                } else {
                    None
                },
                returns_table,
                returns_set,
            },
        );
    }

    Ok(catalog)
}
