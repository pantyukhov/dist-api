//! Hasura v2 GraphQL naming for tracked tables.

use dist_metadata::{QualifiedTable, TableEntry};

/// The base GraphQL name of a table: `custom_name`/custom root field if set,
/// otherwise `<name>` for the `public` schema and `<schema>_<name>` else.
pub fn table_base_name(entry: &TableEntry) -> String {
    if let Some(config) = &entry.configuration {
        if let Some(custom) = &config.custom_name {
            return custom.clone();
        }
    }
    default_base_name(&entry.table)
}

pub fn default_base_name(table: &QualifiedTable) -> String {
    match table.schema() {
        "public" => table.name().to_string(),
        schema => format!("{schema}_{}", table.name()),
    }
}

pub struct RootNames {
    pub select: String,
    pub select_by_pk: String,
    pub select_aggregate: String,
}

pub fn root_names(entry: &TableEntry) -> RootNames {
    let base = table_base_name(entry);
    let custom = entry
        .configuration
        .as_ref()
        .map(|c| &c.custom_root_fields);

    let get = |key: &str, default: String| -> String {
        custom
            .and_then(|m| m.get(key).cloned())
            .unwrap_or(default)
    };

    RootNames {
        select: get("select", base.clone()),
        select_by_pk: get("select_by_pk", format!("{base}_by_pk")),
        select_aggregate: get("select_aggregate", format!("{base}_aggregate")),
    }
}
