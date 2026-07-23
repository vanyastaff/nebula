//! Executable contract for the backend migration catalogs.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fmt::Write as _,
    path::Path,
};

use sha2::{Digest, Sha384};

const HISTORICAL_SHA384: &str = "bee7216c69554a52aad0479eee68d5e46fcb54e4b32fdcb6cd3d2888e7fd3b617f069090b17d856a9cba832b17ee7aef";

#[derive(Debug)]
struct MigrationFile {
    version: u16,
    slug: String,
    file_name: String,
    bytes: Vec<u8>,
}

impl MigrationFile {
    fn parse(file_name: &str, bytes: Vec<u8>) -> Result<Self, String> {
        let (version, slug) = parse_filename(file_name)?;
        Ok(Self {
            version,
            slug: slug.to_owned(),
            file_name: file_name.to_owned(),
            bytes,
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
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            migrations.push(MigrationFile::parse(&file_name, bytes)?);
        }

        Self::from_migrations(backend, migrations)
    }

    fn from_names(backend: &'static str, names: &[&str]) -> Result<Self, String> {
        let migrations = names
            .iter()
            .map(|name| MigrationFile::parse(name, Vec::new()))
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
    let postgres_by_version = postgres.by_version();
    let sqlite_by_version = sqlite.by_version();
    for (version, postgres_migration) in postgres_by_version {
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

fn historical_digest(catalogs: &[&Catalog]) -> String {
    let mut records = catalogs
        .iter()
        .flat_map(|catalog| {
            catalog
                .migrations
                .iter()
                .filter(|migration| migration.version <= 38)
                .map(move |migration| {
                    (
                        format!("{}/{}", catalog.backend, migration.file_name),
                        migration.bytes.as_slice(),
                    )
                })
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digest = Sha384::new();
    for (path, bytes) in records {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    let bytes = digest.finalize();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[test]
fn repository_catalog_matches_k2_contract() {
    let postgres = Catalog::load("postgres").expect("Postgres catalog must be valid");
    let sqlite = Catalog::load("sqlite").expect("SQLite catalog must be valid");

    let expected_postgres = (1_u16..=39).collect::<Vec<_>>();
    let expected_sqlite = (1_u16..=28).chain(30..=35).chain([39]).collect::<Vec<_>>();
    assert_eq!(
        postgres.versions(),
        expected_postgres,
        "Postgres must reserve every logical migration through K2 version 0039"
    );
    assert_eq!(
        sqlite.versions(),
        expected_sqlite,
        "SQLite must contain the shared history and leave PostgreSQL-only versions reserved"
    );

    for reserved in [29, 36, 37, 38] {
        assert!(
            !sqlite.by_version().contains_key(&reserved),
            "PostgreSQL-only migration {reserved:04} must remain absent from SQLite"
        );
    }
    validate_shared_slugs(&postgres, &sqlite).expect("shared migration slugs must match");

    for catalog in [&postgres, &sqlite] {
        let migration = catalog
            .by_version()
            .get(&39)
            .copied()
            .expect("K2 migration 0039 must exist in both backends");
        assert_eq!(
            migration.file_name, "0039_credentials_owner_and_record_state.sql",
            "K2 migration filename is part of the catalog contract"
        );
    }
}

#[test]
fn historical_migration_bytes_are_immutable() {
    let postgres = Catalog::load("postgres").expect("Postgres catalog must be valid");
    let sqlite = Catalog::load("sqlite").expect("SQLite catalog must be valid");

    assert_eq!(
        historical_digest(&[&postgres, &sqlite]),
        HISTORICAL_SHA384,
        "migrations 0001..0038 are immutable; add a new migration instead of editing history"
    );
}

#[test]
fn catalog_parser_rejects_ambiguous_inputs() {
    for malformed in [
        "39_short.sql",
        "0039-Missing-Separator.sql",
        "0039_MixedCase.sql",
        "0039_.sql",
        "0000_zero.sql",
    ] {
        assert!(
            MigrationFile::parse(malformed, Vec::new()).is_err(),
            "accepted malformed migration filename `{malformed}`"
        );
    }

    let duplicate_version =
        Catalog::from_names("synthetic", &["0001_first.sql", "0001_second.sql"]);
    assert!(duplicate_version.is_err(), "accepted a duplicate version");

    let duplicate_slug = Catalog::from_names("synthetic", &["0001_same.sql", "0002_same.sql"]);
    assert!(duplicate_slug.is_err(), "accepted a duplicate slug");

    let postgres = Catalog::from_names("postgres", &["0001_shared.sql"])
        .expect("synthetic Postgres catalog must be valid");
    let sqlite = Catalog::from_names("sqlite", &["0001_conflicting.sql"])
        .expect("synthetic SQLite catalog must be valid");
    assert!(
        validate_shared_slugs(&postgres, &sqlite).is_err(),
        "accepted conflicting slugs for a shared logical version"
    );
}
