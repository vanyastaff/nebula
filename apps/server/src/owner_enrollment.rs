//! Operator-only first account and tenant enrollment, independent of serving.
//!
//! Identity and tenancy commit separately. A durable frozen command bridges the
//! two owners; restart resumes it without replaying a password or restoring grants.

use std::io::{IsTerminal, Read, Write};

use nebula_api::{
    config::{ExecutionBackendKind, ExecutionStoreConfig},
    domain::auth::backend::password,
};
use nebula_core::{OrgId, UserId, WorkspaceId};
use nebula_storage::auth::{InitialOwnerBegin, InitialOwnerRegistration, InitialOwnerStatus};
use nebula_storage_port::dto::{
    PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningRequest,
};
use zeroize::Zeroizing;

use crate::deployment_database::DeploymentDatabase;

#[derive(clap::Subcommand)]
pub(crate) enum SetupCommand {
    /// Create the initial account and organization; read the password from piped stdin.
    Begin {
        #[arg(long)]
        email: String,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        organization_name: String,
    },
    /// Resume the saved enrollment without replacing its account or password.
    Resume,
    /// Show historical enrollment progress, not current access rights.
    Status,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SetupError {
    #[error("setup requires API_EXECUTION_BACKEND=sqlite or postgres")]
    DurableDatabaseRequired,
    #[error("deployment storage configuration is invalid")]
    Configuration,
    #[error(
        "setup account input is invalid: use an email, a password of at least 8 bytes, and names of 1..=128 bytes"
    )]
    InvalidAccount,
    #[error(
        "supply a single password through protected piped stdin (maximum 4096 bytes); terminal echo is not supported"
    )]
    PasswordInput,
    #[error("password hashing failed")]
    PasswordHash,
    #[error("initial enrollment already started; use setup resume or setup status")]
    AlreadyStarted,
    #[error("initial enrollment is permanently closed for this deployment")]
    Sealed,
    #[error("initial enrollment has not started; use setup begin")]
    NotStarted,
    #[error("the pending initial owner account is unavailable; no tenant rights were created")]
    OwnerUnavailable,
    #[error("the saved enrollment conflicts with existing tenant history")]
    Conflict,
    #[error("could not write setup status")]
    Output,
    #[error(transparent)]
    Deployment(#[from] crate::compose::TransportInitError),
    #[error(transparent)]
    Storage(#[from] nebula_storage::StorageError),
}

struct EnrollmentDraft {
    user_id: UserId,
    email: String,
    display_name: String,
    password_hash: Zeroizing<String>,
    request: TenantProvisioningRequest,
}

impl EnrollmentDraft {
    fn prepare(
        email: &str,
        display_name: &str,
        organization_name: &str,
        secret: &str,
    ) -> Result<Self, SetupError> {
        let (email, display_name) = password::validate_registration(email, secret, display_name)
            .map_err(|_| SetupError::InvalidAccount)?;
        let organization_name = organization_name.trim();
        if organization_name.is_empty() || organization_name.len() > 128 {
            return Err(SetupError::InvalidAccount);
        }
        let user_id = UserId::new();
        let org_id = OrgId::new().to_string();
        let workspace_id = WorkspaceId::new().to_string();
        let owner = user_id.to_string();
        let org = TenantOrgCreate::new(
            org_id,
            "personal".into(),
            organization_name.into(),
            owner.clone(),
            "free".into(),
            None,
            serde_json::json!({}),
        )
        .map_err(|_| SetupError::InvalidAccount)?;
        let workspace = TenantDefaultWorkspaceCreate::new(
            workspace_id,
            "default".into(),
            "Default".into(),
            None,
            owner.clone(),
            serde_json::json!({}),
        )
        .map_err(|_| SetupError::InvalidAccount)?;
        let request = TenantProvisioningRequest::new(
            org,
            workspace,
            PrincipalKind::User,
            owner,
            Some("operator-enrollment".into()),
        )
        .map_err(|_| SetupError::InvalidAccount)?;
        let password_hash =
            Zeroizing::new(password::hash_password(secret).map_err(|_| SetupError::PasswordHash)?);
        Ok(Self {
            user_id,
            email,
            display_name: display_name.into(),
            password_hash,
            request,
        })
    }

    fn registration(&self) -> InitialOwnerRegistration<'_> {
        InitialOwnerRegistration {
            user_id: self.user_id,
            email: &self.email,
            display_name: &self.display_name,
            password_hash: &self.password_hash,
            tenant_request: &self.request,
        }
    }
}

fn read_password(reader: impl Read) -> Result<Zeroizing<String>, SetupError> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(4099)
        .read_to_end(&mut bytes)
        .map_err(|_| SetupError::PasswordInput)?;
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    if bytes.is_empty()
        || bytes.len() > 4096
        || bytes.iter().any(|byte| matches!(byte, b'\n' | b'\r' | 0))
    {
        return Err(SetupError::PasswordInput);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| SetupError::PasswordInput)?;
    Ok(Zeroizing::new(text.to_owned()))
}

pub(crate) async fn run(command: SetupCommand) -> Result<(), SetupError> {
    let config = ExecutionStoreConfig::from_env().map_err(|_| SetupError::Configuration)?;
    if config.backend == ExecutionBackendKind::Memory {
        return Err(SetupError::DurableDatabaseRequired);
    }
    // Validate and hash before opening storage; rejected input creates no database.
    let draft = match &command {
        SetupCommand::Begin {
            email,
            display_name,
            organization_name,
        } => {
            let stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err(SetupError::PasswordInput);
            }
            let secret = read_password(stdin.lock())?;
            Some(EnrollmentDraft::prepare(
                email,
                display_name,
                organization_name,
                &secret,
            )?)
        },
        SetupCommand::Resume | SetupCommand::Status => None,
    };
    let database = DeploymentDatabase::open(&config, None).await?;
    if let Some(draft) = draft {
        let result = match &database {
            DeploymentDatabase::Sqlite(deployment) => {
                nebula_storage::auth::sqlite::SqliteAccountLifecycle::new(deployment.pool().clone())
                    .begin_initial_owner(&draft.registration())
                    .await?
            },
            #[cfg(feature = "postgres")]
            DeploymentDatabase::Postgres(pool) => {
                nebula_storage::auth::postgres::PgAccountLifecycle::new(pool.clone())
                    .begin_initial_owner(&draft.registration())
                    .await?
            },
            DeploymentDatabase::Memory(_) => return Err(SetupError::DurableDatabaseRequired),
        };
        match result {
            InitialOwnerBegin::Begun => {},
            InitialOwnerBegin::AlreadyStarted => return Err(SetupError::AlreadyStarted),
            InitialOwnerBegin::Unavailable => return Err(SetupError::Sealed),
        }
    }
    let observing = matches!(command, SetupCommand::Status);
    let status = match &database {
        DeploymentDatabase::Sqlite(deployment) => {
            let tenant = nebula_storage::sqlite::SqliteTenantProvisioningStore::new(
                deployment.pool().clone(),
            );
            if observing {
                tenant.initial_owner_status().await?
            } else {
                tenant.accept_initial_owner().await?
            }
        },
        #[cfg(feature = "postgres")]
        DeploymentDatabase::Postgres(pool) => {
            let tenant = nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone());
            if observing {
                tenant.initial_owner_status().await?
            } else {
                tenant.accept_initial_owner().await?
            }
        },
        DeploymentDatabase::Memory(_) => return Err(SetupError::DurableDatabaseRequired),
    };
    if !observing {
        match status {
            InitialOwnerStatus::Accepted => {},
            InitialOwnerStatus::Available => return Err(SetupError::NotStarted),
            InitialOwnerStatus::Sealed => return Err(SetupError::Sealed),
            InitialOwnerStatus::OwnerUnavailable => return Err(SetupError::OwnerUnavailable),
            InitialOwnerStatus::Conflict | InitialOwnerStatus::Pending => {
                return Err(SetupError::Conflict);
            },
        }
    }
    let label = match status {
        InitialOwnerStatus::Available => "available",
        InitialOwnerStatus::Sealed => "sealed",
        InitialOwnerStatus::Pending => "pending",
        InitialOwnerStatus::OwnerUnavailable => "owner-unavailable",
        InitialOwnerStatus::Accepted => "accepted",
        InitialOwnerStatus::Conflict => "conflict",
    };
    writeln!(std::io::stdout().lock(), "{label}").map_err(|_| SetupError::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_input_is_bounded_and_preserves_intentional_spaces() {
        assert_eq!(
            &*read_password(&b" password with spaces \r\n"[..]).unwrap(),
            " password with spaces "
        );
        for bytes in [
            vec![],
            vec![b'a'; 4097],
            b"first\nsecond".to_vec(),
            b"bad\0secret".to_vec(),
            vec![0xff],
        ] {
            assert!(matches!(
                read_password(bytes.as_slice()),
                Err(SetupError::PasswordInput)
            ));
        }
        assert_eq!(
            read_password(vec![b'a'; 4096].as_slice()).unwrap().len(),
            4096
        );
    }
}
