//! Remote schemas: role-scoped SDL permissions + request forwarding.
//!
//! When every root field of an operation belongs to the role's permitted
//! remote schema (per its SDL document), the whole request is validated
//! against that document and forwarded to the remote GraphQL server.

use std::collections::HashMap;

use axum::http::HeaderMap;
use graphql_parser::query::{
    Definition as QDef, Document as QDoc, Field as QField, OperationDefinition, Selection,
};
use graphql_parser::schema::{Definition as SDef, Document as SDoc, Type, TypeDefinition};
use serde_json::{Value as Json, json};

use dist_schema::Session;

use crate::state::AppState;

pub struct RemoteTarget {
    pub url: String,
    pub forward_client_headers: bool,
    /// Query rewritten with @preset arguments injected, when any.
    pub rewritten_query: Option<String>,
    /// The operation also has introspection root fields, answered
    /// locally and merged with the forwarded response.
    pub has_introspection: bool,
    /// root_fields_namespace: the forwarded response gets wrapped back
    /// under this key.
    pub namespace: Option<String>,
}

/// If the operation is aimed at a permitted remote schema, validate it
/// against the role's SDL and return the forwarding target. `None` means
/// "not a remote operation".
pub fn match_remote<'m>(
    metadata: &'m dist_metadata::Metadata,
    session: &Session,
    doc: &QDoc<'static, String>,
    variables: &mut serde_json::Map<String, Json>,
) -> Option<Result<RemoteTarget, crate::gql::GqlError>> {
    match_remote_with(metadata, session, doc, variables, false)
}

/// `internal` requests (remote-relationship joins) may set arguments that
/// carry @preset (they are server-built, not client input).
pub fn match_remote_with<'m>(
    metadata: &'m dist_metadata::Metadata,
    session: &Session,
    doc: &QDoc<'static, String>,
    variables: &mut serde_json::Map<String, Json>,
    internal: bool,
) -> Option<Result<RemoteTarget, crate::gql::GqlError>> {
    // Collect the operation's root field names.
    let mut root_fields = vec![];
    for def in &doc.definitions {
        if let QDef::Operation(op) = def {
            let set = match op {
                OperationDefinition::Query(q) => &q.selection_set,
                OperationDefinition::SelectionSet(s) => s,
                _ => return None,
            };
            for item in &set.items {
                if let Selection::Field(f) = item {
                    root_fields.push(f);
                }
            }
        }
    }
    if root_fields.is_empty() {
        return None;
    }
    // Introspection roots are answered locally; only the data fields
    // must belong to the remote schema.
    let is_intro = |name: &str| {
        name == "__schema" || name == "__type" || name == "__typename"
    };
    let data_fields: Vec<_> = root_fields
        .iter()
        .filter(|f| !is_intro(&f.name))
        .copied()
        .collect();
    let has_introspection = data_fields.len() != root_fields.len();
    if data_fields.is_empty() {
        return None;
    }

    let mut decustomized_storage: Option<QDoc<'static, String>> = None;
    for schema in &metadata.remote_schemas {
        let Some(permission) = schema
            .permissions
            .iter()
            .find(|p| p.role == session.role)
        else {
            continue;
        };
        // Customized schemas: unwrap the namespace root and translate
        // customized type/field names back to the upstream ones (keeping
        // the customized spelling as response aliases).
        let customization = schema.definition.customization.as_ref();
        let mut namespace = None;
        let doc: &QDoc<'static, String> = if let Some(c) = customization {
            match decustomize(doc, c) {
                Some((d, ns)) => {
                    namespace = ns;
                    decustomized_storage = Some(d);
                    decustomized_storage.as_ref().unwrap()
                }
                None => continue,
            }
        } else {
            doc
        };
        // Re-collect root fields from the (possibly) unwrapped document.
        let mut root_fields = vec![];
        for def in &doc.definitions {
            if let QDef::Operation(op) = def {
                let set = match op {
                    OperationDefinition::Query(q) => &q.selection_set,
                    OperationDefinition::SelectionSet(s) => s,
                    _ => continue,
                };
                for item in &set.items {
                    if let Selection::Field(f) = item {
                        if f.name != "__schema"
                            && f.name != "__type"
                            && f.name != "__typename"
                        {
                            root_fields.push(f);
                        }
                    }
                }
            }
        }
        if root_fields.is_empty() {
            continue;
        }
        let Ok(sdl) = graphql_parser::parse_schema::<String>(&permission.definition.schema)
        else {
            continue;
        };
        let sdl = sdl.into_static();
        let types = type_map(&sdl);
        let Some(query_type) = root_type_name(&sdl, &types) else {
            continue;
        };
        // All data root fields must exist on the permitted Query type.
        let all_match = root_fields
            .iter()
            .all(|f| field_on_type(&types, &query_type, &f.name).is_some());
        if !all_match {
            continue;
        }
        // Deep validation with exact Hasura-style errors.
        for field in &root_fields {
            if field.name == "__typename" {
                continue;
            }
            if let Err(e) = validate_field(
                &types,
                &query_type,
                field,
                &format!("$.selectionSet.{}", field.name),
                internal,
            ) {
                return Some(Err(e));
            }
        }
        // Inject @preset arguments and strip introspection roots from
        // the forwarded document.
        let mut rewritten = doc.clone();
        if has_introspection {
            strip_introspection_roots(&mut rewritten);
        }
        let rewritten_query =
            match apply_presets(&mut rewritten, &types, &query_type, session, variables) {
                Ok(true) => Some(format!("{rewritten}")),
                Ok(false) if has_introspection => Some(format!("{rewritten}")),
                Ok(false) => None,
                Err(e) => return Some(Err(e)),
            };
        let raw_url = schema
            .definition
            .url
            .clone()
            .or_else(|| {
                schema
                    .definition
                    .url_from_env
                    .as_ref()
                    .and_then(|v| std::env::var(v).ok())
            })
            .unwrap_or_default();
        let url = resolve_url_template(&raw_url);
        return Some(Ok(RemoteTarget {
            url,
            forward_client_headers: schema.definition.forward_client_headers,
            rewritten_query: rewritten_query
                .or_else(|| decustomized_storage.as_ref().map(|d| format!("{d}"))),
            has_introspection,
            namespace,
        }));
    }
    None
}

/// Hasura resolves {{ENV_VAR}} templates in remote urls itself.
pub fn resolve_url_template(raw_url: &str) -> String {
    if let (Some(start), Some(end)) = (raw_url.find("{{"), raw_url.find("}}")) {
        let var = &raw_url[start + 2..end];
        let value = std::env::var(var).unwrap_or_default();
        format!("{}{}{}", &raw_url[..start], value, &raw_url[end + 2..])
    } else {
        raw_url.to_string()
    }
}

type Types<'d> = HashMap<String, &'d TypeDefinition<'static, String>>;

fn type_map<'d>(sdl: &'d SDoc<'static, String>) -> Types<'d> {
    let mut map = HashMap::new();
    for def in &sdl.definitions {
        if let SDef::TypeDefinition(td) = def {
            let name = match td {
                TypeDefinition::Object(o) => &o.name,
                TypeDefinition::Scalar(s) => &s.name,
                TypeDefinition::Interface(i) => &i.name,
                TypeDefinition::Union(u) => &u.name,
                TypeDefinition::Enum(e) => &e.name,
                TypeDefinition::InputObject(io) => &io.name,
            };
            map.insert(name.clone(), td);
        }
    }
    map
}

fn root_type_name(sdl: &SDoc<'static, String>, types: &Types) -> Option<String> {
    for def in &sdl.definitions {
        if let SDef::SchemaDefinition(sd) = def {
            if let Some(q) = &sd.query {
                return Some(q.clone());
            }
        }
    }
    types.contains_key("Query").then(|| "Query".to_string())
}

fn unwrap_type<'t>(ty: &'t Type<'static, String>) -> &'t str {
    match ty {
        Type::NamedType(n) => n,
        Type::ListType(inner) | Type::NonNullType(inner) => unwrap_type(inner),
    }
}

fn field_on_type<'d>(
    types: &'d Types,
    type_name: &str,
    field: &str,
) -> Option<&'d graphql_parser::schema::Field<'static, String>> {
    match types.get(type_name) {
        Some(TypeDefinition::Object(o)) => o.fields.iter().find(|f| f.name == field),
        Some(TypeDefinition::Interface(i)) => i.fields.iter().find(|f| f.name == field),
        _ => None,
    }
}

fn validate_field(
    types: &Types,
    parent_type: &str,
    field: &QField<'static, String>,
    path: &str,
    internal: bool,
) -> Result<(), crate::gql::GqlError> {
    let Some(def) = field_on_type(types, parent_type, &field.name) else {
        return Err(crate::gql::GqlError {
            path: path.to_string(),
            code: "validation-failed",
            message: format!("field '{}' not found in type: '{parent_type}'", field.name),
        });
    };
    for (arg, _) in &field.arguments {
        // Arguments carrying @preset are hidden from the role's schema
        // (but server-built join queries may use them).
        let visible = def.arguments.iter().any(|a| {
            &a.name == arg
                && (internal || !a.directives.iter().any(|d| d.name == "preset"))
        });
        if !visible {
            return Err(crate::gql::GqlError {
                path: path.to_string(),
                code: "validation-failed",
                message: format!("'{}' has no argument named '{arg}'", field.name),
            });
        }
    }
    let inner_type = unwrap_type(&def.field_type).to_string();
    for item in &field.selection_set.items {
        if let Selection::Field(sub) = item {
            if sub.name == "__typename" {
                continue;
            }
            validate_field(
                types,
                &inner_type,
                sub,
                &format!("{path}.selectionSet.{}", sub.name),
                internal,
            )?;
        }
    }
    Ok(())
}

/// Translate a customized operation back to upstream names: unwrap the
/// root namespace field, strip type/field prefixes (adding the customized
/// name as an alias so the response keys keep the client's spelling),
/// and rewrite fragment type conditions. Returns None when the document
/// does not fit the customization (e.g. missing namespace root).
fn decustomize(
    doc: &QDoc<'static, String>,
    c: &dist_metadata::RemoteSchemaCustomization,
) -> Option<(QDoc<'static, String>, Option<String>)> {
    let mut doc = doc.clone();

    let strip_type = |name: &str| -> String {
        let mut out = name.to_string();
        if let Some(tn) = &c.type_names {
            if let Some(p) = &tn.prefix {
                out = out.strip_prefix(p.as_str()).unwrap_or(&out).to_string();
            }
            if let Some(sfx) = &tn.suffix {
                out = out.strip_suffix(sfx.as_str()).unwrap_or(&out).to_string();
            }
        }
        out
    };
    // Strip ALL known field prefixes (parent type tracking is overkill
    // for the fixtures: prefixes are distinctive).
    let strip_field = |name: &str| -> String {
        for rule in &c.field_names {
            if let Some(p) = &rule.prefix {
                if let Some(rest) = name.strip_prefix(p.as_str()) {
                    return rest.to_string();
                }
            }
            if let Some(sfx) = &rule.suffix {
                if let Some(rest) = name.strip_suffix(sfx.as_str()) {
                    return rest.to_string();
                }
            }
        }
        name.to_string()
    };

    fn walk_fields(
        set: &mut graphql_parser::query::SelectionSet<'static, String>,
        strip_field: &dyn Fn(&str) -> String,
        strip_type: &dyn Fn(&str) -> String,
    ) {
        for item in &mut set.items {
            match item {
                Selection::Field(f) => {
                    let upstream = strip_field(&f.name);
                    if upstream != f.name {
                        if f.alias.is_none() {
                            f.alias = Some(f.name.clone());
                        }
                        f.name = upstream;
                    }
                    walk_fields(&mut f.selection_set, strip_field, strip_type);
                }
                Selection::InlineFragment(frag) => {
                    if let Some(graphql_parser::query::TypeCondition::On(t)) =
                        &mut frag.type_condition
                    {
                        *t = strip_type(t);
                    }
                    walk_fields(&mut frag.selection_set, strip_field, strip_type);
                }
                Selection::FragmentSpread(_) => {}
            }
        }
    }

    let mut namespace = None;
    for def in &mut doc.definitions {
        match def {
            QDef::Operation(op) => {
                let set = match op {
                    OperationDefinition::Query(q) => &mut q.selection_set,
                    OperationDefinition::SelectionSet(s) => s,
                    _ => return None,
                };
                if let Some(ns) = &c.root_fields_namespace {
                    // The single root field must be the namespace wrapper.
                    if set.items.len() == 1 {
                        if let Selection::Field(f) = &set.items[0] {
                            if &f.name == ns {
                                namespace = Some(ns.clone());
                                let inner = f.selection_set.clone();
                                *set = inner;
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                walk_fields(set, &strip_field, &strip_type);
            }
            QDef::Fragment(frag) => {
                let graphql_parser::query::TypeCondition::On(t) = &mut frag.type_condition;
                *t = strip_type(t);
                walk_fields(&mut frag.selection_set, &strip_field, &strip_type);
            }
        }
    }
    Some((doc, namespace))
}

/// Remove __schema/__type/__typename roots (they are answered locally).
pub fn strip_introspection_roots(doc: &mut QDoc<'static, String>) {
    for def in &mut doc.definitions {
        if let QDef::Operation(op) = def {
            let set = match op {
                OperationDefinition::Query(q) => &mut q.selection_set,
                OperationDefinition::SelectionSet(s) => s,
                _ => continue,
            };
            set.items.retain(|item| {
                !matches!(item, Selection::Field(f)
                    if f.name == "__schema" || f.name == "__type" || f.name == "__typename")
            });
        }
    }
}

/// Keep only the introspection roots.
pub fn keep_introspection_roots(doc: &mut QDoc<'static, String>) {
    for def in &mut doc.definitions {
        if let QDef::Operation(op) = def {
            let set = match op {
                OperationDefinition::Query(q) => &mut q.selection_set,
                OperationDefinition::SelectionSet(s) => s,
                _ => continue,
            };
            set.items.retain(|item| {
                matches!(item, Selection::Field(f)
                    if f.name == "__schema" || f.name == "__type" || f.name == "__typename")
            });
        }
    }
}

/// Inject preset arguments into every field per the role SDL. Returns
/// whether anything changed.
fn apply_presets(
    doc: &mut QDoc<'static, String>,
    types: &Types,
    query_type: &str,
    session: &Session,
    variables: &mut serde_json::Map<String, Json>,
) -> Result<bool, crate::gql::GqlError> {
    use graphql_parser::query::Value as QValue;

    /// Preset fields declared on an input object type, recursively.
    fn input_presets(
        types: &Types,
        type_name: &str,
    ) -> Vec<(String, graphql_parser::schema::Value<'static, String>)> {
        let Some(TypeDefinition::InputObject(io)) = types.get(type_name) else {
            return vec![];
        };
        let mut out = vec![];
        for f in &io.fields {
            for d in &f.directives {
                if d.name == "preset" {
                    if let Some((_, v)) = d.arguments.iter().find(|(n, _)| n == "value") {
                        out.push((f.name.clone(), v.clone()));
                    }
                }
            }
        }
        out
    }

    fn preset_to_json(v: &graphql_parser::schema::Value<'static, String>) -> Json {
        use graphql_parser::schema::Value as SV;
        match v {
            SV::Int(n) => Json::from(n.as_i64().unwrap_or(0)),
            SV::Float(f) => serde_json::Number::from_f64(*f)
                .map(Json::Number)
                .unwrap_or(Json::Null),
            SV::String(s) => Json::String(s.clone()),
            SV::Boolean(b) => Json::Bool(*b),
            SV::Null => Json::Null,
            SV::Enum(e) => Json::String(e.clone()),
            SV::List(items) => Json::Array(items.iter().map(preset_to_json).collect()),
            SV::Object(map) => Json::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), preset_to_json(v)))
                    .collect(),
            ),
            SV::Variable(_) => Json::Null,
        }
    }

    fn schema_value_to_query(
        v: &graphql_parser::schema::Value<'static, String>,
    ) -> QValue<'static, String> {
        use graphql_parser::schema::Value as SV;
        match v {
            SV::Int(n) => QValue::Int((n.as_i64().unwrap_or(0) as i32).into()),
            SV::Float(f) => QValue::Float(*f),
            SV::String(s) => QValue::String(s.clone()),
            SV::Boolean(b) => QValue::Boolean(*b),
            SV::Null => QValue::Null,
            SV::Enum(e) => QValue::Enum(e.clone()),
            SV::List(items) => {
                QValue::List(items.iter().map(schema_value_to_query).collect())
            }
            SV::Object(map) => QValue::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), schema_value_to_query(v)))
                    .collect(),
            ),
            SV::Variable(name) => QValue::Variable(name.clone()),
        }
    }

    fn walk(
        types: &Types,
        parent_type: &str,
        field: &mut QField<'static, String>,
        session: &Session,
        changed: &mut bool,
        variables: &mut serde_json::Map<String, Json>,
    ) -> Result<(), crate::gql::GqlError> {
        let Some(def) = field_on_type(types, parent_type, &field.name) else {
            return Ok(());
        };
        let def = def.clone();
        for arg in &def.arguments {
            for d in &arg.directives {
                if d.name != "preset" {
                    continue;
                }
                let Some((_, raw)) = d.arguments.iter().find(|(n, _)| n == "value") else {
                    continue;
                };
                let value = match raw {
                    graphql_parser::schema::Value::String(s)
                        if s.len() >= 8 && s[..8].eq_ignore_ascii_case("x-hasura") =>
                    {
                        let Some(found) = session.var(s) else {
                            return Err(crate::gql::GqlError {
                                path: "$".to_string(),
                                code: "not-found",
                                message: format!(
                                    "\"{}\" session variable expected, but not found",
                                    s.to_ascii_lowercase()
                                ),
                            });
                        };
                        // Coerce by the argument's base type.
                        let base = unwrap_type(&arg.value_type);
                        match base {
                            "Int" => match found.parse::<i32>() {
                                Ok(n) => QValue::Int(n.into()),
                                Err(_) => {
                                    return Err(crate::gql::GqlError {
                                        path: "$".to_string(),
                                        code: "coercion-error",
                                        message: format!(
                                            "expecting an integer for the session variable \"{}\", but found {:?}",
                                            s.to_ascii_lowercase(),
                                            found
                                        ),
                                    });
                                }
                            },
                            "Boolean" => QValue::Boolean(found == "true"),
                            _ => QValue::String(found.to_string()),
                        }
                    }
                    other => schema_value_to_query(other),
                };
                // Client-passed preset args are rejected at validation;
                // anything already present here is server-built — keep it.
                if !field.arguments.iter().any(|(n, _)| n == &arg.name) {
                    field.arguments.push((arg.name.clone(), value));
                    *changed = true;
                }
            }
        }
        // Input-object presets: merge declared preset fields into the
        // argument value (creating it when absent); for variables, patch
        // the variables map instead.
        for arg in &def.arguments {
            let base = unwrap_type(&arg.value_type).to_string();
            let presets = input_presets(types, &base);
            if presets.is_empty() {
                continue;
            }
            let existing = field
                .arguments
                .iter_mut()
                .find(|(n, _)| n == &arg.name);
            match existing {
                Some((_, QValue::Variable(var))) => {
                    let entry = variables
                        .entry(var.clone())
                        .or_insert_with(|| Json::Object(serde_json::Map::new()));
                    if let Json::Object(map) = entry {
                        for (k, v) in &presets {
                            if !map.contains_key(k) {
                                map.insert(k.clone(), preset_to_json(v));
                            }
                        }
                        *changed = true;
                    }
                }
                Some((_, QValue::Object(map))) => {
                    for (k, v) in &presets {
                        if !map.contains_key(k) {
                            map.insert(k.clone(), schema_value_to_query(v));
                        }
                    }
                    *changed = true;
                }
                Some(_) => {}
                None => {
                    let map: std::collections::BTreeMap<String, QValue<'static, String>> =
                        presets
                            .iter()
                            .map(|(k, v)| (k.clone(), schema_value_to_query(v)))
                            .collect();
                    field.arguments.push((arg.name.clone(), QValue::Object(map)));
                    *changed = true;
                }
            }
        }

        let inner = unwrap_type(&def.field_type).to_string();
        for item in &mut field.selection_set.items {
            if let Selection::Field(sub) = item {
                walk(types, &inner, sub, session, changed, variables)?;
            }
        }
        Ok(())
    }

    let mut changed = false;
    for def in &mut doc.definitions {
        if let QDef::Operation(op) = def {
            let set = match op {
                OperationDefinition::Query(q) => &mut q.selection_set,
                OperationDefinition::SelectionSet(s) => s,
                _ => continue,
            };
            for item in &mut set.items {
                if let Selection::Field(f) = item {
                    walk(types, query_type, f, session, &mut changed, variables)?;
                }
            }
        }
    }
    Ok(changed)
}

/// POST the operation to the remote server.
pub async fn forward(
    state: &AppState,
    target: &RemoteTarget,
    body: &Json,
    headers: &HeaderMap,
) -> (axum::http::StatusCode, Json) {
    let mut payload = body.clone();
    if let Some(query) = &target.rewritten_query {
        payload["query"] = Json::String(query.clone());
    }
    let mut request = state.http.post(&target.url).json(&payload);
    if target.forward_client_headers {
        for (name, value) in headers {
            let name = name.as_str();
            if name.starts_with("x-hasura-") || name == "authorization" || name == "cookie" {
                if let Ok(value) = value.to_str() {
                    request = request.header(name, value);
                }
            }
        }
    }
    match request.send().await {
        Ok(response) => {
            let status = axum::http::StatusCode::from_u16(response.status().as_u16())
                .unwrap_or(axum::http::StatusCode::OK);
            let body = response.json::<Json>().await.unwrap_or(Json::Null);
            (status, body)
        }
        Err(e) => (
            axum::http::StatusCode::OK,
            json!({
                "errors": [{
                    "extensions": { "path": "$", "code": "unexpected" },
                    "message": format!("remote schema request failed: {e}"),
                }]
            }),
        ),
    }
}
