use crate::{DiagnosticKind, DiagnosticsRecorder, HarnessError, PrincipalId, ResourceKind};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityRule {
    pub principal: PrincipalId,
    pub operation: String,
    pub resource: String,
    pub allow: bool,
}

impl CapabilityRule {
    pub fn new(
        principal: PrincipalId,
        operation: impl Into<String>,
        resource: impl Into<String>,
        allow: bool,
    ) -> Self {
        Self {
            principal,
            operation: operation.into(),
            resource: resource.into(),
            allow,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub reason: &'static str,
}

#[derive(Clone, Default)]
pub struct FakeCapabilityBroker {
    rules: BTreeMap<(PrincipalId, String, String), bool>,
    diagnostics: Option<DiagnosticsRecorder>,
}

impl FakeCapabilityBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_diagnostics(mut self, diagnostics: DiagnosticsRecorder) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    pub fn add_rule(&mut self, rule: CapabilityRule) -> crate::Result<()> {
        if rule.operation.is_empty()
            || rule.resource.is_empty()
            || rule.operation.len() > 128
            || rule.resource.len() > 128
        {
            return Err(HarnessError::InvalidConfiguration(
                "capability operation and resource must contain 1 to 128 bytes".to_owned(),
            ));
        }
        if !self.rules.contains_key(&(
            rule.principal.clone(),
            rule.operation.clone(),
            rule.resource.clone(),
        )) && self.rules.len() >= 256
        {
            return Err(HarnessError::QuotaExceeded(ResourceKind::Handles));
        }
        self.rules
            .insert((rule.principal, rule.operation, rule.resource), rule.allow);
        Ok(())
    }

    pub fn authorize(
        &self,
        principal: Option<&PrincipalId>,
        operation: &str,
        resource: &str,
    ) -> AuthorizationDecision {
        let decision = match principal {
            None => AuthorizationDecision {
                allowed: false,
                reason: "missing-context",
            },
            Some(principal) => match self.rules.get(&(
                principal.clone(),
                operation.to_owned(),
                resource.to_owned(),
            )) {
                Some(true) => AuthorizationDecision {
                    allowed: true,
                    reason: "explicit-allow",
                },
                Some(false) => AuthorizationDecision {
                    allowed: false,
                    reason: "explicit-deny",
                },
                None => AuthorizationDecision {
                    allowed: false,
                    reason: "no-matching-rule",
                },
            },
        };
        if let Some(diagnostics) = &self.diagnostics {
            let mut fields = BTreeMap::new();
            fields.insert("allowed".to_owned(), decision.allowed.to_string());
            fields.insert("operation".to_owned(), operation.to_owned());
            fields.insert(
                "principal".to_owned(),
                principal
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<missing>".to_owned()),
            );
            fields.insert("reason".to_owned(), decision.reason.to_owned());
            fields.insert("resource".to_owned(), resource.to_owned());
            diagnostics.record(
                DiagnosticKind::AuthorizationDecision,
                None,
                "authorize",
                fields,
            );
        }
        decision
    }

    pub fn check(
        &self,
        principal: Option<&PrincipalId>,
        operation: &str,
        resource: &str,
    ) -> crate::Result<()> {
        let decision = self.authorize(principal, operation, resource);
        if decision.allowed {
            Ok(())
        } else if decision.reason == "missing-context" {
            Err(HarnessError::MissingContext)
        } else {
            Err(HarnessError::Unauthorized)
        }
    }
}
