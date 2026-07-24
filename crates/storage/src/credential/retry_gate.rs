use chrono::{DateTime, Utc};
use nebula_storage_port::{
    CredentialMaterialTransition, CredentialPersistenceError, RefreshRetryAdmission,
    RefreshRetryBlock, RefreshRetryDelay, RefreshRetryDiagnosticCode, RefreshRetryEvidence,
    RefreshRetryGate, RefreshRetryKind, RefreshRetryPhase, RefreshRetryTransition,
};

pub(crate) const MODE_NEVER: &str = "never";
pub(crate) const MODE_NOT_BEFORE: &str = "not_before";
pub(crate) const TRANSITION_PRESERVE: i16 = 0;
pub(crate) const TRANSITION_CLEAR: i16 = 1;
pub(crate) const TRANSITION_SET_NEVER: i16 = 2;
pub(crate) const TRANSITION_SET_AFTER: i16 = 3;

pub(crate) struct EncodedTransition<'a> {
    pub(crate) code: i16,
    pub(crate) delay_seconds: Option<i64>,
    pub(crate) phase: Option<&'static str>,
    pub(crate) kind: Option<&'static str>,
    pub(crate) diagnostic_code: Option<&'a str>,
}

pub(crate) fn encode_transition(
    transition: &RefreshRetryTransition,
) -> Result<EncodedTransition<'_>, CredentialPersistenceError> {
    let encoded = match transition {
        RefreshRetryTransition::Preserve => EncodedTransition {
            code: TRANSITION_PRESERVE,
            delay_seconds: None,
            phase: None,
            kind: None,
            diagnostic_code: None,
        },
        RefreshRetryTransition::Clear => EncodedTransition {
            code: TRANSITION_CLEAR,
            delay_seconds: None,
            phase: None,
            kind: None,
            diagnostic_code: None,
        },
        RefreshRetryTransition::SetNever { evidence } => EncodedTransition {
            code: TRANSITION_SET_NEVER,
            delay_seconds: None,
            phase: Some(phase_code(evidence.phase())),
            kind: Some(kind_code(evidence.kind())),
            diagnostic_code: evidence
                .diagnostic_code()
                .map(RefreshRetryDiagnosticCode::as_str),
        },
        RefreshRetryTransition::SetAfter { delay, evidence } => EncodedTransition {
            code: TRANSITION_SET_AFTER,
            delay_seconds: Some(
                i64::try_from(delay.as_secs())
                    .map_err(|_| CredentialPersistenceError::CorruptRecord)?,
            ),
            phase: Some(phase_code(evidence.phase())),
            kind: Some(kind_code(evidence.kind())),
            diagnostic_code: evidence
                .diagnostic_code()
                .map(RefreshRetryDiagnosticCode::as_str),
        },
    };
    Ok(encoded)
}

/// Encode the gate half of an explicit material-authority transition.
///
/// Advancing material always encodes `Clear`; callers cannot attach an
/// old-epoch gate because that combination is absent from the port type.
pub(crate) fn encode_material_transition(
    transition: &CredentialMaterialTransition,
) -> Result<EncodedTransition<'_>, CredentialPersistenceError> {
    match transition {
        CredentialMaterialTransition::Preserve { refresh_retry } => {
            encode_transition(refresh_retry)
        },
        CredentialMaterialTransition::Advance => Ok(EncodedTransition {
            code: TRANSITION_CLEAR,
            delay_seconds: None,
            phase: None,
            kind: None,
            diagnostic_code: None,
        }),
    }
}

pub(crate) fn phase_code(phase: RefreshRetryPhase) -> &'static str {
    match phase {
        RefreshRetryPhase::BeforeDispatch => "before_dispatch",
        RefreshRetryPhase::ProviderConfirmedNotApplied => "provider_confirmed_not_applied",
    }
}

pub(crate) fn kind_code(kind: RefreshRetryKind) -> &'static str {
    match kind {
        RefreshRetryKind::TransientNetwork => "transient_network",
        RefreshRetryKind::ProviderUnavailable => "provider_unavailable",
        RefreshRetryKind::ProtocolError => "protocol_error",
    }
}

pub(crate) fn decode_evidence(
    phase: &str,
    kind: &str,
    diagnostic_code: Option<String>,
) -> Result<RefreshRetryEvidence, CredentialPersistenceError> {
    let phase = match phase {
        "before_dispatch" => RefreshRetryPhase::BeforeDispatch,
        "provider_confirmed_not_applied" => RefreshRetryPhase::ProviderConfirmedNotApplied,
        _ => return Err(CredentialPersistenceError::CorruptRecord),
    };
    let kind = match kind {
        "transient_network" => RefreshRetryKind::TransientNetwork,
        "provider_unavailable" => RefreshRetryKind::ProviderUnavailable,
        "protocol_error" => RefreshRetryKind::ProtocolError,
        _ => return Err(CredentialPersistenceError::CorruptRecord),
    };
    let diagnostic_code = diagnostic_code
        .map(RefreshRetryDiagnosticCode::parse)
        .transpose()
        .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
    Ok(RefreshRetryEvidence::new(phase, kind, diagnostic_code))
}

pub(crate) fn decode_gate(
    mode: Option<String>,
    not_before: Option<DateTime<Utc>>,
    phase: Option<String>,
    kind: Option<String>,
    diagnostic_code: Option<String>,
) -> Result<Option<RefreshRetryGate>, CredentialPersistenceError> {
    match (mode, not_before, phase, kind, diagnostic_code) {
        (None, None, None, None, None) => Ok(None),
        (Some(mode), not_before, Some(phase), Some(kind), diagnostic_code) => {
            let evidence = decode_evidence(&phase, &kind, diagnostic_code)?;
            match (mode.as_str(), not_before) {
                (MODE_NEVER, None) => Ok(Some(RefreshRetryGate::Never { evidence })),
                (MODE_NOT_BEFORE, Some(not_before)) => Ok(Some(RefreshRetryGate::NotBefore {
                    not_before,
                    evidence,
                })),
                _ => Err(CredentialPersistenceError::CorruptRecord),
            }
        },
        _ => Err(CredentialPersistenceError::CorruptRecord),
    }
}

pub(crate) fn evaluate_gate(
    gate: Option<&RefreshRetryGate>,
    now: DateTime<Utc>,
) -> Result<RefreshRetryAdmission, CredentialPersistenceError> {
    match gate {
        None => Ok(RefreshRetryAdmission::Open),
        Some(RefreshRetryGate::Never { evidence }) => {
            Ok(RefreshRetryAdmission::Blocked(RefreshRetryBlock::Never {
                evidence: evidence.clone(),
            }))
        },
        Some(RefreshRetryGate::NotBefore {
            not_before,
            evidence,
        }) if *not_before <= now => Ok(RefreshRetryAdmission::Open),
        Some(RefreshRetryGate::NotBefore {
            not_before,
            evidence,
        }) => {
            let remaining = not_before
                .signed_duration_since(now)
                .to_std()
                .map_err(|_| CredentialPersistenceError::CorruptRecord)
                .and_then(|duration| {
                    RefreshRetryDelay::new(duration)
                        .map_err(|_| CredentialPersistenceError::CorruptRecord)
                })?;
            Ok(RefreshRetryAdmission::Blocked(RefreshRetryBlock::After {
                remaining,
                evidence: evidence.clone(),
            }))
        },
    }
}

#[cfg(test)]
pub(crate) fn apply_transition(
    current: Option<&RefreshRetryGate>,
    transition: &RefreshRetryTransition,
    now: DateTime<Utc>,
) -> Result<Option<RefreshRetryGate>, CredentialPersistenceError> {
    match transition {
        RefreshRetryTransition::Preserve => Ok(current.cloned()),
        RefreshRetryTransition::Clear => Ok(None),
        RefreshRetryTransition::SetNever { evidence } => Ok(Some(RefreshRetryGate::Never {
            evidence: evidence.clone(),
        })),
        RefreshRetryTransition::SetAfter { delay, evidence } => {
            let seconds = i64::try_from(delay.as_secs())
                .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
            let not_before = now
                .checked_add_signed(chrono::Duration::seconds(seconds))
                .ok_or(CredentialPersistenceError::CorruptRecord)?;
            Ok(Some(RefreshRetryGate::NotBefore {
                not_before,
                evidence: evidence.clone(),
            }))
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use nebula_storage_port::{
        CredentialPersistenceError, RefreshRetryAdmission, RefreshRetryBlock, RefreshRetryEvidence,
        RefreshRetryGate, RefreshRetryKind, RefreshRetryPhase,
    };

    use super::{decode_gate, evaluate_gate};

    fn evidence() -> RefreshRetryEvidence {
        RefreshRetryEvidence::new(
            RefreshRetryPhase::BeforeDispatch,
            RefreshRetryKind::TransientNetwork,
            None,
        )
    }

    #[test]
    fn persisted_codec_rejects_unknown_and_incomplete_tuples() {
        for result in [
            decode_gate(
                Some("future".to_owned()),
                None,
                Some("before_dispatch".to_owned()),
                Some("transient_network".to_owned()),
                None,
            ),
            decode_gate(
                Some("never".to_owned()),
                None,
                None,
                Some("transient_network".to_owned()),
                None,
            ),
            decode_gate(
                Some("not_before".to_owned()),
                None,
                Some("before_dispatch".to_owned()),
                Some("transient_network".to_owned()),
                None,
            ),
            decode_gate(
                Some("never".to_owned()),
                None,
                Some("before_dispatch".to_owned()),
                Some("future_kind".to_owned()),
                None,
            ),
            decode_gate(
                Some("never".to_owned()),
                None,
                Some("before_dispatch".to_owned()),
                Some("transient_network".to_owned()),
                Some("free form".to_owned()),
            ),
        ] {
            assert_eq!(result, Err(CredentialPersistenceError::CorruptRecord));
        }
    }

    #[test]
    fn admission_rounds_remaining_up_and_opens_expired_gate() {
        let now = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("test timestamp");
        let future = RefreshRetryGate::NotBefore {
            not_before: now + chrono::Duration::milliseconds(1_001),
            evidence: evidence(),
        };
        assert!(matches!(
            evaluate_gate(Some(&future), now),
            Ok(RefreshRetryAdmission::Blocked(RefreshRetryBlock::After {
                remaining,
                ..
            })) if remaining.as_secs() == 2
        ));

        let expired = RefreshRetryGate::NotBefore {
            not_before: now,
            evidence: evidence(),
        };
        assert_eq!(
            evaluate_gate(Some(&expired), now),
            Ok(RefreshRetryAdmission::Open)
        );
    }
}
