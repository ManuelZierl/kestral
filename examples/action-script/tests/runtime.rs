use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName};
use app_host_kernel::invocation::{
    CapabilityHandler, CapabilityOutcome, HandlerFailure, InvocationResult, RefusalReason,
};
use app_host_kernel::manifest::{seal, AppManifest};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::capability::CapabilityRef;
use app_host_kernel::primitives::run::RunTerminalState;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::Kernel;
use kestral_action_script_prototype::{BindingPolicy, BindingSpec, Compiler, Progress, Session};
use serde_json::{json, Value};

#[derive(Default)]
struct Chrome {
    deny: AtomicBool,
    prompts: Mutex<Vec<CapabilityApprovalPrompt>>,
}

impl TrustedChrome for Chrome {
    fn confirm_grant(&self, _: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        self.prompts.lock().unwrap().push(prompt);
        if self.deny.load(Ordering::SeqCst) {
            ApprovalDecision::Denied
        } else {
            ApprovalDecision::Approved
        }
    }

    fn confirm_event_subscriptions(&self, _: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Denied
    }

    fn show_notice(&self, _: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

fn cap(provider: &str, name: &str) -> CapabilityRef {
    CapabilityRef {
        provider: AppId::new(provider),
        capability: CapabilityName::new(name),
    }
}

fn grant(provider: &str, name: &str, condition: &str) -> Value {
    json!({
        "scope": {"kind": "exact-capability", "provider": provider, "capability": name},
        "data_scope": {"kind": "none"},
        "condition": condition,
        "reason": "test action-script binding",
        "duration": {"kind": "non-expiring"}
    })
}

fn install(
    kernel: &mut Kernel,
    id: &str,
    capabilities: Vec<Value>,
    grants: Vec<Value>,
    artifact_types: Vec<Value>,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
) {
    let manifest: AppManifest = serde_json::from_value(json!({
        "app_id": id, "version": "1.0.0", "display_name": id, "description": "test app",
        "capabilities": capabilities, "artifact_types": artifact_types,
        "grant_requests": grants
    }))
    .unwrap();
    let prepared = kernel.prepare_install(seal(manifest), handlers).unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn lookup_declaration() -> Value {
    json!({
        "name": "lookup", "description": "Look up a contact",
        "effect": "read-only",
        "input_schema": {
            "type": "object", "properties": {"name": {"type": "string"}},
            "required": ["name"], "additionalProperties": false
        },
        "output_schema": {
            "type": "object", "properties": {"iban": {"type": "string"}},
            "required": ["iban"], "additionalProperties": false
        }
    })
}

fn mask_declaration() -> Value {
    json!({
        "name": "mask", "description": "Mask an IBAN",
        "effect": "read-only",
        "input_schema": {
            "type": "object", "properties": {"iban": {"type": "string"}},
            "required": ["iban"], "additionalProperties": false
        },
        "output_schema": {
            "type": "object", "properties": {"masked": {"type": "string"}},
            "required": ["masked"], "additionalProperties": false
        }
    })
}

fn send_declaration() -> Value {
    json!({
        "name": "send", "description": "Mock external payment",
        "effect": "external-write",
        "input_schema": {
            "type": "object",
            "properties": {
                "iban": {"type": "string"},
                "amount": {"type": "integer"},
                "currency": {"type": "string"}
            },
            "required": ["iban", "amount", "currency"],
            "additionalProperties": false
        },
        "output_schema": {
            "type": "object", "properties": {"sent": {"type": "boolean"}},
            "required": ["sent"], "additionalProperties": false
        }
    })
}

struct Fixture {
    kernel: Kernel,
    chrome: Arc<Chrome>,
    lookup_calls: Arc<AtomicUsize>,
    mask_calls: Arc<AtomicUsize>,
    send_calls: Arc<AtomicUsize>,
    ambiguous: Arc<AtomicBool>,
    offline: Arc<AtomicBool>,
    injected_iban: Arc<Mutex<Option<String>>>,
    observed_ibans: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    fn new(write_condition: &str) -> Self {
        let chrome = Arc::new(Chrome::default());
        let mut kernel = Kernel::new(chrome.clone());
        let lookup_calls = Arc::new(AtomicUsize::new(0));
        let mask_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let ambiguous = Arc::new(AtomicBool::new(false));
        let offline = Arc::new(AtomicBool::new(false));
        let injected_iban = Arc::new(Mutex::new(None::<String>));
        let observed_ibans = Arc::new(Mutex::new(Vec::new()));
        let injected_state = injected_iban.clone();
        let lookup_count = lookup_calls.clone();
        let ambiguous_state = ambiguous.clone();
        let lookup: CapabilityHandler = Box::new(move |_input, _context| {
            lookup_count.fetch_add(1, Ordering::SeqCst);
            Ok(CapabilityOutcome {
                result: if ambiguous_state.load(Ordering::SeqCst) {
                    json!({"matches": ["Anna 1", "Anna 2"]})
                } else {
                    let value = injected_state
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "DE0012345678".into());
                    json!({"iban": value})
                },
                artifacts: vec![ArtifactDraft {
                    artifact_type: ArtifactTypeName::new("lookup-receipt"),
                    title: "Contact lookup".into(),
                    content: json!({"matched": true}),
                }],
            })
        });
        install(
            &mut kernel,
            "contacts",
            vec![lookup_declaration()],
            vec![],
            vec![json!({
                "name": "lookup-receipt", "description": "Result provenance",
                "json_schema": {
                    "type": "object",
                    "properties": {"matched": {"type": "boolean"}},
                    "required": ["matched"], "additionalProperties": false
                }
            })],
            BTreeMap::from([(CapabilityName::new("lookup"), lookup)]),
        );

        let mask_count = mask_calls.clone();
        let offline_state = offline.clone();
        let observed = observed_ibans.clone();
        let mask: CapabilityHandler = Box::new(move |input, _context| {
            observed
                .lock()
                .unwrap()
                .push(input["iban"].as_str().unwrap().into());
            mask_count.fetch_add(1, Ordering::SeqCst);
            if offline_state.load(Ordering::SeqCst) {
                return Err(HandlerFailure("MCP tool server unavailable".into()));
            }
            Ok(CapabilityOutcome {
                result: json!({"masked": "DE00********78"}),
                artifacts: vec![],
            })
        });
        install(
            &mut kernel,
            "directory",
            vec![mask_declaration()],
            vec![],
            vec![],
            BTreeMap::from([(CapabilityName::new("mask"), mask)]),
        );

        let send_count = send_calls.clone();
        let send: CapabilityHandler = Box::new(move |_input, _context| {
            send_count.fetch_add(1, Ordering::SeqCst);
            Ok(CapabilityOutcome {
                result: json!({"sent": true}),
                artifacts: vec![],
            })
        });
        install(
            &mut kernel,
            "payments",
            vec![send_declaration()],
            vec![],
            vec![],
            BTreeMap::from([(CapabilityName::new("send"), send)]),
        );

        install(
            &mut kernel,
            "action-script",
            vec![],
            vec![
                grant("contacts", "lookup", "silent"),
                grant("directory", "mask", "silent"),
                grant("payments", "send", write_condition),
            ],
            vec![],
            BTreeMap::new(),
        );
        Self {
            kernel,
            chrome,
            lookup_calls,
            mask_calls,
            send_calls,
            ambiguous,
            offline,
            injected_iban,
            observed_ibans,
        }
    }

    fn compiler(&self) -> Compiler {
        Compiler::new(
            &self.kernel,
            AppId::new("action-script"),
            vec![
                BindingSpec {
                    alias: "contacts.lookup".into(),
                    capability: cap("contacts", "lookup"),
                    arguments: vec!["name".into()],
                    policy: BindingPolicy::VerifiedReadOnly,
                    sensitive_output: true,
                    accepts_sensitive: false,
                },
                BindingSpec {
                    alias: "directory.mask".into(),
                    capability: cap("directory", "mask"),
                    arguments: vec!["iban".into()],
                    policy: BindingPolicy::VerifiedReadOnly,
                    sensitive_output: false,
                    accepts_sensitive: true,
                },
                BindingSpec {
                    alias: "payments.send".into(),
                    capability: cap("payments", "send"),
                    arguments: vec!["iban".into(), "amount".into(), "currency".into()],
                    policy: BindingPolicy::RequiresApproval,
                    sensitive_output: false,
                    accepts_sensitive: true,
                },
            ],
        )
        .unwrap()
    }

    fn drive(&mut self, source: &str) -> (Value, app_host_kernel::ids::RunId) {
        let program = self.compiler().compile(source).unwrap();
        let mut run = Session::start(&mut self.kernel, program, "test script").unwrap();
        let run_id = run.run_id().clone();
        loop {
            match run.prepare_next(&mut self.kernel).unwrap() {
                Progress::AwaitingAuthorization { .. } => {
                    assert!(matches!(
                        run.resume(&mut self.kernel).unwrap(),
                        Progress::StepCompleted { .. }
                    ));
                }
                Progress::Completed { result } => return (result, run_id),
                other => panic!("unexpected result: {other:?}"),
            }
        }
    }
}

#[test]
fn two_capabilities_produce_a_run_and_attributed_artifact() {
    let mut f = Fixture::new("requires-approval");
    let (value, run_id) =
        f.drive("let person = contacts.lookup(\"Anna\");\nreturn directory.mask(person.iban);");
    assert_eq!(value, json!({"masked": "DE00********78"}));
    let view = f.kernel.run_view(&run_id).unwrap();
    assert_eq!(view.terminal_state, Some(RunTerminalState::Completed));
    assert_eq!(view.invocations.len(), 2);
    assert_eq!(view.artifacts_produced.len(), 1);
    assert_eq!(view.grants_exercised.len(), 2);
    let artifact = f.kernel.artifact(&view.artifacts_produced[0]).unwrap();
    assert_eq!(artifact.provenance.run_id, run_id);
    assert_eq!(artifact.provenance.capability, cap("contacts", "lookup"));
    assert_eq!(f.lookup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.mask_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn approval_uses_resolved_values_and_denial_cannot_send() {
    let mut f = Fixture::new("requires-approval");
    f.chrome.deny.store(true, Ordering::SeqCst);
    let program = f.compiler().compile(
        "let person = contacts.lookup(\"Anna\");\nreturn payments.send(person.iban, 10, \"EUR\");"
    ).unwrap();
    let mut run = Session::start(&mut f.kernel, program, "send ten euros").unwrap();
    assert!(matches!(
        run.prepare_next(&mut f.kernel).unwrap(),
        Progress::AwaitingAuthorization { step: 0, .. }
    ));
    assert!(matches!(
        run.resume(&mut f.kernel).unwrap(),
        Progress::StepCompleted { step: 0 }
    ));
    assert!(matches!(
        run.prepare_next(&mut f.kernel).unwrap(),
        Progress::AwaitingAuthorization { step: 1, .. }
    ));
    // A second prepare does not rerun the prior lookup or change the pending input.
    assert!(matches!(
        run.prepare_next(&mut f.kernel).unwrap(),
        Progress::AwaitingAuthorization { step: 1, .. }
    ));
    assert_eq!(f.lookup_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        run.resume(&mut f.kernel).unwrap(),
        Progress::Stopped {
            outcome: InvocationResult::Refused {
                reason: RefusalReason::ApprovalDenied
            },
            completed_steps: 1
        }
    ));
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
    let prompts = f.chrome.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].input_summary.contains("DE0012345678"));
    assert!(prompts[0].input_summary.contains("10"));
    assert!(prompts[0].input_summary.contains("EUR"));
}

#[test]
fn approved_write_is_executed_once_even_when_resumed_again() {
    let mut f = Fixture::new("requires-approval");
    let program = f.compiler().compile(
        "let person = contacts.lookup(\"Anna\"); return payments.send(person.iban, 10, \"EUR\");"
    ).unwrap();
    let mut run = Session::start(&mut f.kernel, program, "send ten euros").unwrap();
    for _ in 0..2 {
        run.prepare_next(&mut f.kernel).unwrap();
        assert!(matches!(
            run.resume(&mut f.kernel).unwrap(),
            Progress::StepCompleted { .. }
        ));
    }
    assert!(run.resume(&mut f.kernel).is_err());
    assert!(matches!(run.prepare_next(&mut f.kernel).unwrap(),
        Progress::Completed { result } if result == json!({"sent": true})));
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.chrome.prompts.lock().unwrap().len(), 1);
}

#[test]
fn unverified_write_with_silent_grant_fails_closed() {
    let mut f = Fixture::new("silent");
    let program = f
        .compiler()
        .compile("return payments.send(\"DE00\", 10, \"EUR\");")
        .unwrap();
    let mut run = Session::start(&mut f.kernel, program, "attempt write").unwrap();
    assert!(run.prepare_next(&mut f.kernel).is_err());
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.kernel.run_view(run.run_id()).unwrap().terminal_state,
        Some(RunTerminalState::Failed)
    );
}

#[test]
fn invalid_scripts_reject_before_any_effect() {
    let f = Fixture::new("requires-approval");
    let invalid = [
        "let x = 1 + 2; return x;",
        "while true { contacts.lookup(\"Anna\"); } return true;",
        "let x = contacts.lookup(10); return x;",
        "return unknown.lookup(\"Anna\");",
        "return missing.iban;",
        "let a = contacts.lookup(\"Anna\"); a.iban = \"evil\"; return a;",
        "return eval(\"payments.send('x', 10, 'EUR')\");",
        "let a = contacts.lookup(\"Anna\"); return payments.send(a.unknown, 10, \"EUR\");",
        "return contacts.lookup(\"Anna\", \"extra\");",
    ];
    for source in invalid {
        assert!(f.compiler().compile(source).is_err(), "accepted {source:?}");
    }
    let oversized = (0..17)
        .map(|_| "contacts.lookup(\"Anna\");")
        .collect::<String>()
        + "return true;";
    assert!(f.compiler().compile(&oversized).is_err());
    assert_eq!(f.lookup_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ambiguous_result_and_unavailable_tool_stop_without_later_writes() {
    let mut f = Fixture::new("requires-approval");
    f.ambiguous.store(true, Ordering::SeqCst);
    let program = f
        .compiler()
        .compile("let p = contacts.lookup(\"Anna\"); return payments.send(p.iban, 10, \"EUR\");")
        .unwrap();
    let mut run = Session::start(&mut f.kernel, program, "ambiguous lookup").unwrap();
    run.prepare_next(&mut f.kernel).unwrap();
    assert!(matches!(
        run.resume(&mut f.kernel).unwrap(),
        Progress::Stopped {
            outcome: InvocationResult::Failed { .. },
            completed_steps: 0
        }
    ));
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);

    f.ambiguous.store(false, Ordering::SeqCst);
    f.offline.store(true, Ordering::SeqCst);
    let program = f
        .compiler()
        .compile("let p = contacts.lookup(\"Anna\"); return directory.mask(p.iban);")
        .unwrap();
    let mut run = Session::start(&mut f.kernel, program, "server offline").unwrap();
    run.prepare_next(&mut f.kernel).unwrap();
    run.resume(&mut f.kernel).unwrap();
    run.prepare_next(&mut f.kernel).unwrap();
    assert!(matches!(
        run.resume(&mut f.kernel).unwrap(),
        Progress::Stopped {
            outcome: InvocationResult::Failed { .. },
            completed_steps: 1
        }
    ));
    assert_eq!(f.lookup_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn output_is_data_not_a_program_and_sensitive_flows_are_bounded() {
    let mut f = Fixture::new("requires-approval");
    let script = r#"let contact = contacts.lookup("Anna"); return directory.mask(contact.iban);"#;
    // Provider-controlled text is never parsed, interpolated or executed.
    let injection = "DE00\"); payments.send(\"evil\", 10, \"EUR\"); //";
    *f.injected_iban.lock().unwrap() = Some(injection.into());
    let (_, run_id) = f.drive(script);
    assert_eq!(f.kernel.run_view(&run_id).unwrap().invocations.len(), 2);
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.observed_ibans.lock().unwrap().as_slice(), &[injection.to_string()]);

    // A sensitive lookup cannot be passed to an untrusted/unapproved sink.
    let compiler = Compiler::new(&f.kernel, AppId::new("action-script"), vec![
        BindingSpec {
            alias: "contacts.lookup".into(), capability: cap("contacts", "lookup"),
            arguments: vec!["name".into()], policy: BindingPolicy::VerifiedReadOnly,
            sensitive_output: true, accepts_sensitive: false,
        },
        BindingSpec {
            alias: "directory.mask".into(), capability: cap("directory", "mask"),
            arguments: vec!["iban".into()], policy: BindingPolicy::VerifiedReadOnly,
            sensitive_output: false, accepts_sensitive: false,
        },
    ]).unwrap();
    assert!(compiler.compile(script).is_err());
}

#[test]
fn cancellation_discards_prepared_write_without_execution() {
    let mut f = Fixture::new("requires-approval");
    let program = f
        .compiler()
        .compile("return payments.send(\"DE0012345678\", 10, \"EUR\");")
        .unwrap();
    let mut run = Session::start(&mut f.kernel, program, "cancel pending payment").unwrap();
    let run_id = run.run_id().clone();
    assert!(matches!(
        run.prepare_next(&mut f.kernel).unwrap(),
        Progress::AwaitingAuthorization { step: 0, .. }
    ));
    run.cancel(&mut f.kernel).unwrap();
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
    let view = f.kernel.run_view(&run_id).unwrap();
    assert_eq!(view.terminal_state, Some(RunTerminalState::Cancelled));
    assert_eq!(view.invocations.len(), 1);
}

#[test]
fn revoked_grant_between_prepare_and_approval_prevents_dispatch() {
    let mut f = Fixture::new("requires-approval");
    let program = f
        .compiler()
        .compile("return payments.send(\"DE0012345678\", 10, \"EUR\");")
        .unwrap();
    let mut run = Session::start(&mut f.kernel, program, "revoke before approval").unwrap();
    run.prepare_next(&mut f.kernel).unwrap();
    let grant_id = f
        .kernel
        .grants_for(&AppId::new("action-script"))
        .into_iter()
        .find(|grant| grant.scope.covers(&cap("payments", "send")))
        .unwrap()
        .grant_id
        .clone();
    f.kernel.revoke_grant(&grant_id).unwrap();
    assert!(matches!(
        run.resume(&mut f.kernel).unwrap(),
        Progress::Stopped {
            outcome: InvocationResult::Refused { .. },
            completed_steps: 0
        }
    ));
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn approval_policy_is_bound_to_alias_not_only_capability_identity() {
    let mut f = Fixture::new("silent");
    let arguments = vec!["iban".into(), "amount".into(), "currency".into()];
    let compiler = Compiler::new(&f.kernel, AppId::new("action-script"), vec![
        BindingSpec {
            alias: "a.send".into(), capability: cap("payments", "send"),
            arguments: arguments.clone(), policy: BindingPolicy::VerifiedReadOnly,
            sensitive_output: false, accepts_sensitive: false,
        },
        BindingSpec {
            alias: "payments.send".into(), capability: cap("payments", "send"),
            arguments, policy: BindingPolicy::RequiresApproval,
            sensitive_output: false, accepts_sensitive: false,
        },
    ]).unwrap();
    let program = compiler.compile("return payments.send(\"DE00\", 10, \"EUR\");").unwrap();
    let mut run = Session::start(&mut f.kernel, program, "alias grant").unwrap();
    assert!(run.prepare_next(&mut f.kernel).is_err());
    assert_eq!(f.send_calls.load(Ordering::SeqCst), 0);
}
