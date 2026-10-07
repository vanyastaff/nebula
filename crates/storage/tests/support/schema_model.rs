//! A dialect-neutral model of a migrated schema, read back by introspection,
//! so the SQLite and PostgreSQL catalogs can be compared structurally.
//!
//! Types are compared by storage family, not by name: PostgreSQL's richer
//! types (`JSONB`, `TIMESTAMPTZ`, `BOOLEAN`, …) are stored by SQLite as `TEXT`
//! or `INTEGER`. Index and constraint names are ignored; their column lists,
//! uniqueness and partiality are compared.

use std::collections::{BTreeMap, BTreeSet};

/// How a column's values are stored, across dialects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Family {
    Bytes,
    Text,
    Integer,
    Real,
    /// A PostgreSQL instant; the database standard requires SQLite epoch
    /// microseconds in an `INTEGER`, never a textual timestamp.
    Time,
}

impl Family {
    fn matches(self, other: Self) -> bool {
        match (self, other) {
            (Self::Time, Self::Integer) | (Self::Integer, Self::Time) => true,
            _ => self == other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Column {
    pub(crate) family: Family,
    pub(crate) not_null: bool,
}

impl Column {
    fn matches(&self, other: &Self) -> bool {
        self.not_null == other.not_null && self.family.matches(other.family)
    }
}

/// An index or unique constraint, by what it covers. Expression keys are
/// recorded as `<expr>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Index {
    pub(crate) columns: Vec<String>,
    pub(crate) unique: bool,
    pub(crate) partial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ForeignKey {
    pub(crate) columns: Vec<String>,
    pub(crate) references: String,
    pub(crate) referenced_columns: Vec<String>,
    pub(crate) on_delete: String,
    pub(crate) on_update: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Table {
    pub(crate) columns: BTreeMap<String, Column>,
    pub(crate) primary_key: Vec<String>,
    pub(crate) indexes: BTreeSet<Index>,
    pub(crate) foreign_keys: BTreeSet<ForeignKey>,
}

pub(crate) type Schema = BTreeMap<String, Table>;

/// Every structural difference between `left` and `right`, one line each,
/// ignoring the tables named in `left_only` (present only on the left).
pub(crate) fn differences(
    left_name: &str,
    left: &Schema,
    right_name: &str,
    right: &Schema,
    left_only: &BTreeSet<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let names: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    for name in names {
        match (left.get(name), right.get(name)) {
            (Some(_), None) if left_only.contains(name.as_str()) => {},
            (Some(_), None) => out.push(format!("table {name}: only in {left_name}")),
            (None, Some(_)) => out.push(format!("table {name}: only in {right_name}")),
            (Some(l), Some(r)) => {
                if left_only.contains(name.as_str()) {
                    out.push(format!(
                        "table {name}: listed {left_name}-only but present in {right_name}"
                    ));
                }
                table_differences(name, left_name, l, right_name, r, &mut out);
            },
            (None, None) => {},
        }
    }
    out
}

fn table_differences(
    table: &str,
    left_name: &str,
    left: &Table,
    right_name: &str,
    right: &Table,
    out: &mut Vec<String>,
) {
    let columns: BTreeSet<&String> = left.columns.keys().chain(right.columns.keys()).collect();
    for column in columns {
        match (left.columns.get(column), right.columns.get(column)) {
            (Some(l), Some(r)) if !l.matches(r) => {
                out.push(format!(
                    "{table}.{column}: {left_name} {l:?} vs {right_name} {r:?}"
                ));
            },
            (Some(_), None) => out.push(format!("{table}.{column}: only in {left_name}")),
            (None, Some(_)) => out.push(format!("{table}.{column}: only in {right_name}")),
            _ => {},
        }
    }
    if left.primary_key != right.primary_key {
        out.push(format!(
            "{table}: primary key {left_name} {:?} vs {right_name} {:?}",
            left.primary_key, right.primary_key
        ));
    }
    for index in left.indexes.symmetric_difference(&right.indexes) {
        let side = if left.indexes.contains(index) {
            left_name
        } else {
            right_name
        };
        out.push(format!("{table}: index {index:?} only in {side}"));
    }
    for key in left.foreign_keys.symmetric_difference(&right.foreign_keys) {
        let side = if left.foreign_keys.contains(key) {
            left_name
        } else {
            right_name
        };
        out.push(format!("{table}: foreign key {key:?} only in {side}"));
    }
}

/// SQLite storage family from a declared column type (type affinity rules).
fn sqlite_family(declared: &str) -> Family {
    let declared = declared.to_ascii_uppercase();
    if declared.contains("INT") || declared.contains("BOOL") {
        Family::Integer
    } else if declared.contains("CHAR") || declared.contains("TEXT") || declared.contains("CLOB") {
        Family::Text
    } else if declared.contains("BLOB") || declared.is_empty() {
        Family::Bytes
    } else if declared.contains("REAL") || declared.contains("FLOA") || declared.contains("DOUB") {
        Family::Real
    } else {
        Family::Text
    }
}

/// PostgreSQL storage family from `pg_type.typname`.
fn postgres_family(type_name: &str) -> Family {
    match type_name {
        "bytea" => Family::Bytes,
        "int2" | "int4" | "int8" | "bool" => Family::Integer,
        "float4" | "float8" | "numeric" => Family::Real,
        "timestamptz" | "timestamp" | "date" => Family::Time,
        _ => Family::Text,
    }
}

#[cfg(feature = "sqlite")]
pub(crate) async fn sqlite_schema(pool: &sqlx::SqlitePool) -> Schema {
    use sqlx::Row;

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("list SQLite tables");
    let mut schema = Schema::new();
    for name in tables {
        let mut table = Table::default();
        let mut primary_key = Vec::new();
        for row in sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT name, type, \"notnull\", pk FROM pragma_table_info('{name}')"
        )))
        .fetch_all(pool)
        .await
        .expect("SQLite table_info")
        {
            let column: String = row.get("name");
            let declared: String = row.get("type");
            let pk: i64 = row.get("pk");
            if pk > 0 {
                primary_key.push((pk, column.clone()));
            }
            table.columns.insert(
                column,
                Column {
                    family: sqlite_family(&declared),
                    not_null: row.get::<i64, _>("notnull") != 0,
                },
            );
        }
        primary_key.sort();
        table.primary_key = primary_key.into_iter().map(|(_, column)| column).collect();
        // A lone `INTEGER PRIMARY KEY` aliases the rowid and can never hold
        // NULL, although SQLite reports it as nullable.
        if let [only] = table.primary_key.as_slice()
            && let Some(column) = table.columns.get_mut(only)
            && column.family == Family::Integer
        {
            column.not_null = true;
        }

        for row in sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT name, \"unique\", origin, partial FROM pragma_index_list('{name}')"
        )))
        .fetch_all(pool)
        .await
        .expect("SQLite index_list")
        {
            let index: String = row.get("name");
            let origin: String = row.get("origin");
            if origin == "pk" {
                continue;
            }
            let columns: Vec<Option<String>> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT name FROM pragma_index_info('{index}') ORDER BY seqno"
            )))
            .fetch_all(pool)
            .await
            .expect("SQLite index_info");
            table.indexes.insert(Index {
                columns: columns
                    .into_iter()
                    .map(|column| column.unwrap_or_else(|| "<expr>".to_owned()))
                    .collect(),
                unique: row.get::<i64, _>("unique") != 0,
                partial: row.get::<i64, _>("partial") != 0,
            });
        }

        let mut keys: BTreeMap<i64, ForeignKey> = BTreeMap::new();
        for row in sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT id, \"table\", \"from\", \"to\", on_delete, on_update FROM pragma_foreign_key_list('{name}') \
             ORDER BY id, seq"
        )))
        .fetch_all(pool)
        .await
        .expect("SQLite foreign_key_list")
        {
            let key = keys.entry(row.get("id")).or_insert_with(|| ForeignKey {
                columns: Vec::new(),
                references: row.get("table"),
                referenced_columns: Vec::new(),
                on_delete: row.get("on_delete"),
                on_update: row.get("on_update"),
            });
            key.columns.push(row.get("from"));
            key.referenced_columns
                .push(row.get::<Option<String>, _>("to").unwrap_or_default());
        }
        table.foreign_keys = keys.into_values().collect();
        schema.insert(name, table);
    }

    // `REFERENCES parent` without a column list targets the parent's primary
    // key; SQLite reports those referenced columns as NULL.
    let primary_keys: BTreeMap<String, Vec<String>> = schema
        .iter()
        .map(|(name, table)| (name.clone(), table.primary_key.clone()))
        .collect();
    for table in schema.values_mut() {
        table.foreign_keys = std::mem::take(&mut table.foreign_keys)
            .into_iter()
            .map(|mut key| {
                if key.referenced_columns.iter().all(String::is_empty) {
                    key.referenced_columns = primary_keys
                        .get(&key.references)
                        .cloned()
                        .unwrap_or_default();
                }
                key
            })
            .collect();
    }
    schema
}

#[cfg(feature = "postgres")]
pub(crate) async fn postgres_schema(pool: &sqlx::PgPool) -> Schema {
    use sqlx::Row;

    let mut schema = Schema::new();
    for row in sqlx::query(
        "SELECT c.relname AS table_name, a.attname AS column_name, t.typname AS type_name, \
                a.attnotnull AS not_null \
         FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
         JOIN pg_type t ON t.oid = a.atttypid \
         WHERE n.nspname = current_schema() AND c.relkind = 'r' \
           AND c.relname <> '_sqlx_migrations'",
    )
    .fetch_all(pool)
    .await
    .expect("list PostgreSQL columns")
    {
        let type_name: String = row.get("type_name");
        schema
            .entry(row.get("table_name"))
            .or_default()
            .columns
            .insert(
                row.get("column_name"),
                Column {
                    family: postgres_family(&type_name),
                    not_null: row.get("not_null"),
                },
            );
    }

    for row in sqlx::query(
        "SELECT c.relname AS table_name, \
                ARRAY(SELECT COALESCE(a.attname, '<expr>') \
                      FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
                      LEFT JOIN pg_attribute a \
                        ON a.attrelid = c.oid AND a.attnum = k.attnum AND k.attnum > 0 \
                      ORDER BY k.ord)::text[] AS columns, \
                i.indisunique AS is_unique, i.indisprimary AS is_primary, \
                i.indpred IS NOT NULL AS is_partial \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = current_schema() AND c.relname <> '_sqlx_migrations'",
    )
    .fetch_all(pool)
    .await
    .expect("list PostgreSQL indexes")
    {
        let table = schema.entry(row.get("table_name")).or_default();
        let columns: Vec<String> = row.get("columns");
        if row.get::<bool, _>("is_primary") {
            table.primary_key = columns;
        } else {
            table.indexes.insert(Index {
                columns,
                unique: row.get("is_unique"),
                partial: row.get("is_partial"),
            });
        }
    }

    for row in sqlx::query(
        "SELECT c.relname AS table_name, r.relname AS referenced, \
                ARRAY(SELECT a.attname FROM unnest(k.conkey) WITH ORDINALITY AS x(attnum, ord) \
                      JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.attnum \
                      ORDER BY x.ord)::text[] AS columns, \
                ARRAY(SELECT a.attname FROM unnest(k.confkey) WITH ORDINALITY AS x(attnum, ord) \
                      JOIN pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = x.attnum \
                      ORDER BY x.ord)::text[] AS referenced_columns, \
                CASE k.confdeltype WHEN 'a' THEN 'NO ACTION' WHEN 'r' THEN 'RESTRICT' \
                     WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' END AS on_delete, \
                CASE k.confupdtype WHEN 'a' THEN 'NO ACTION' WHEN 'r' THEN 'RESTRICT' \
                     WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' END AS on_update \
         FROM pg_constraint k \
         JOIN pg_class c ON c.oid = k.conrelid \
         JOIN pg_class r ON r.oid = k.confrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = current_schema() AND k.contype = 'f'",
    )
    .fetch_all(pool)
    .await
    .expect("list PostgreSQL foreign keys")
    {
        schema
            .entry(row.get("table_name"))
            .or_default()
            .foreign_keys
            .insert(ForeignKey {
                columns: row.get("columns"),
                references: row.get("referenced"),
                referenced_columns: row.get("referenced_columns"),
                on_delete: row.get("on_delete"),
                on_update: row.get("on_update"),
            });
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::{Family, ForeignKey, Schema, Table, differences};
    use std::collections::BTreeSet;

    #[test]
    fn a_textual_timestamp_does_not_satisfy_the_database_standard() {
        assert!(Family::Time.matches(Family::Integer));
        assert!(!Family::Time.matches(Family::Text));
    }

    #[test]
    fn a_cascade_difference_is_not_schema_parity() {
        let key = ForeignKey {
            columns: vec!["parent_id".into()],
            references: "parents".into(),
            referenced_columns: vec!["id".into()],
            on_delete: "CASCADE".into(),
            on_update: "NO ACTION".into(),
        };
        let left = Schema::from([(
            "children".into(),
            Table {
                foreign_keys: BTreeSet::from([key.clone()]),
                ..Table::default()
            },
        )]);
        let right = Schema::from([(
            "children".into(),
            Table {
                foreign_keys: BTreeSet::from([ForeignKey {
                    on_delete: "RESTRICT".into(),
                    ..key
                }]),
                ..Table::default()
            },
        )]);
        assert!(!differences("postgres", &left, "sqlite", &right, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn an_exception_cannot_hide_a_shared_table() {
        let schema = Schema::from([("replay_cache".into(), Table::default())]);
        let failures = differences(
            "postgres",
            &schema,
            "sqlite",
            &schema,
            &BTreeSet::from(["replay_cache"]),
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("listed postgres-only but present in sqlite"));
    }
}
