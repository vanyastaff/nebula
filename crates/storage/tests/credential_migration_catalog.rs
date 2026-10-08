//! Executable contract for the baseline and append-only backend catalogs.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    path::Path,
};

use nebula_storage::migration_catalog::REVIEWED_HEAD;

const BASELINE: [&str; 8] = [
    "0001_identity.sql",
    "0002_tenancy.sql",
    "0003_workflows.sql",
    "0004_executions.sql",
    "0005_dispatch.sql",
    "0006_credentials.sql",
    "0007_resources.sql",
    "0008_platform.sql",
];

#[derive(Debug)]
struct MigrationFile {
    version: u16,
    slug: String,
    file_name: String,
}

impl MigrationFile {
    fn parse(file_name: &str) -> Result<Self, String> {
        let (version, slug) = parse_filename(file_name)?;
        Ok(Self {
            version,
            slug: slug.to_owned(),
            file_name: file_name.to_owned(),
        })
    }
}

#[derive(Debug)]
struct Catalog {
    backend: &'static str,
    migrations: Vec<MigrationFile>,
}

impl Catalog {
    fn load(backend: &'static str) -> Result<Self, String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .join(backend);
        let entries = std::fs::read_dir(&root)
            .map_err(|error| format!("read {}: {error}", root.display()))?;
        let mut migrations = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("read {} entry: {error}", root.display()))?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|error| format!("inspect {}: {error}", path.display()))?
                .is_file()
                || path.extension() != Some(OsStr::new("sql"))
            {
                continue;
            }
            let file_name = entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 migration filename".to_owned())?;
            migrations.push(MigrationFile::parse(&file_name)?);
        }
        Self::from_migrations(backend, migrations)
    }

    fn from_names(backend: &'static str, names: &[&str]) -> Result<Self, String> {
        let migrations = names
            .iter()
            .map(|name| MigrationFile::parse(name))
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_migrations(backend, migrations)
    }

    fn from_migrations(
        backend: &'static str,
        mut migrations: Vec<MigrationFile>,
    ) -> Result<Self, String> {
        let mut versions = BTreeSet::new();
        let mut slugs = BTreeSet::new();
        for migration in &migrations {
            if !versions.insert(migration.version) {
                return Err(format!(
                    "{backend} has duplicate migration version {:04}",
                    migration.version
                ));
            }
            if !slugs.insert(migration.slug.clone()) {
                return Err(format!(
                    "{backend} has duplicate migration slug `{}`",
                    migration.slug
                ));
            }
        }
        migrations.sort_by_key(|migration| migration.version);
        Ok(Self {
            backend,
            migrations,
        })
    }

    fn versions(&self) -> Vec<u16> {
        self.migrations
            .iter()
            .map(|migration| migration.version)
            .collect()
    }

    fn by_version(&self) -> BTreeMap<u16, &MigrationFile> {
        self.migrations
            .iter()
            .map(|migration| (migration.version, migration))
            .collect()
    }
}

fn parse_filename(file_name: &str) -> Result<(u16, &str), String> {
    let stem = file_name
        .strip_suffix(".sql")
        .ok_or_else(|| format!("migration filename must end in `.sql`: {file_name}"))?;
    let (digits, slug) = stem
        .split_once('_')
        .ok_or_else(|| format!("migration filename must be `NNNN_slug.sql`: {file_name}"))?;
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "migration version must be exactly four decimal digits: {file_name}"
        ));
    }
    if slug.is_empty()
        || slug.starts_with('_')
        || slug.ends_with('_')
        || slug.contains("__")
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!(
            "migration slug must be lowercase snake case: {file_name}"
        ));
    }
    let version = digits
        .parse::<u16>()
        .map_err(|error| format!("invalid migration version in {file_name}: {error}"))?;
    if version == 0 {
        return Err(format!("migration version must be positive: {file_name}"));
    }
    Ok((version, slug))
}

fn validate_shared_slugs(postgres: &Catalog, sqlite: &Catalog) -> Result<(), String> {
    let sqlite_by_version = sqlite.by_version();
    for (version, postgres_migration) in postgres.by_version() {
        let Some(sqlite_migration) = sqlite_by_version.get(&version) else {
            continue;
        };
        if postgres_migration.slug != sqlite_migration.slug {
            return Err(format!(
                "shared migration {version:04} has conflicting slugs: postgres=`{}`, sqlite=`{}`",
                postgres_migration.slug, sqlite_migration.slug
            ));
        }
    }
    Ok(())
}

#[test]
fn repository_catalog_has_baseline_and_reviewed_append_only_sequence() {
    let postgres = Catalog::load("postgres").expect("Postgres catalog must be valid");
    let sqlite = Catalog::load("sqlite").expect("SQLite catalog must be valid");
    let head = u16::try_from(REVIEWED_HEAD).expect("reviewed head fits u16");
    assert!(
        usize::from(head) >= BASELINE.len(),
        "reviewed head must include the whole baseline"
    );
    let expected_versions = (1_u16..=head).collect::<Vec<_>>();
    for catalog in [&postgres, &sqlite] {
        assert_eq!(
            catalog.versions(),
            expected_versions,
            "{} catalog must be contiguous through the reviewed head",
            catalog.backend
        );
        let names = catalog
            .migrations
            .iter()
            .take(BASELINE.len())
            .map(|migration| migration.file_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names, BASELINE,
            "{} must retain the named baseline prefix",
            catalog.backend
        );
    }
    validate_shared_slugs(&postgres, &sqlite).expect("shared migration slugs must match");
}

#[test]
fn readme_inventory_matches_every_migration_file() {
    for backend in ["postgres", "sqlite"] {
        let catalog = Catalog::load(backend).expect("catalog must be valid");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .join(backend)
            .join("README.md");
        let readme = std::fs::read_to_string(path).expect("migration README must exist");
        let documented = readme
            .lines()
            .filter_map(|line| {
                let row = line.strip_prefix("| `")?;
                let (name, _) = row.split_once("` |")?;
                Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
                    .then_some(name.to_owned())
            })
            .collect::<Vec<_>>();
        let actual = catalog
            .migrations
            .iter()
            .map(|migration| migration.file_name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            documented, actual,
            "{backend} README must document each migration exactly once in catalog order"
        );
    }
}

#[test]
fn catalog_parser_rejects_ambiguous_inputs() {
    for malformed in [
        "1_short.sql",
        "0001-Missing-Separator.sql",
        "0001_MixedCase.sql",
        "0001_.sql",
        "0001_two__words.sql",
        "0000_zero.sql",
        "0001_missing_extension",
    ] {
        assert!(
            MigrationFile::parse(malformed).is_err(),
            "accepted malformed migration filename `{malformed}`"
        );
    }
    assert!(
        Catalog::from_names("synthetic", &["0001_first.sql", "0001_second.sql"]).is_err(),
        "accepted a duplicate version"
    );
    assert!(
        Catalog::from_names("synthetic", &["0001_same.sql", "0002_same.sql"]).is_err(),
        "accepted a duplicate slug"
    );
    let postgres =
        Catalog::from_names("postgres", &["0001_shared.sql"]).expect("synthetic catalog valid");
    let sqlite =
        Catalog::from_names("sqlite", &["0001_conflicting.sql"]).expect("synthetic catalog valid");
    assert!(
        validate_shared_slugs(&postgres, &sqlite).is_err(),
        "accepted conflicting shared slugs"
    );
}
