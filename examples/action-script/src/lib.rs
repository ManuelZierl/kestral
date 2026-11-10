//! Restricted Kestral action scripts. This is a userland runtime adapter, NOT a
//! kernel service, Rhai interpreter, or a new source of capability authority.
//!
//! The Rhai parser is deliberately never used to execute a script. Only this
//! module's narrow, serializable IR is interpreted, one kernel invocation at
//! a time. The binding table is supplied by the host, never by model output.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use app_host_kernel::ids::{AppId, RunId};
use app_host_kernel::invocation::{InvocationRequest, InvocationResult};
use app_host_kernel::kernel::{AuthorizeInvocation, PrepareInvocation, PreparedInvocation};
use app_host_kernel::primitives::capability::{CapabilityDeclaration, CapabilityRef};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition};
use app_host_kernel::primitives::run::{Initiator, RunTerminalState};
use app_host_kernel::{JsonObject, Kernel, KernelError};
use rhai::{ASTFlags, Engine, Expr, FnCallExpr, OptimizationLevel, Stmt};
use serde::Serialize;
use serde_json::{json, Value};

pub const LANGUAGE: &str = "kestral-action-1";
const MAX_SOURCE_BYTES: usize = 8 * 1024;
const MAX_NODES: usize = 256;
const MAX_STEPS: usize = 16;
const MAX_ARGUMENTS: usize = 16;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_TOTAL_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_RUN_TIME: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("script rejected: {0}")]
    Rejected(String),
    #[error("script execution failed: {0}")]
    Execution(String),
    #[error(transparent)]
    Kernel(#[from] KernelError),
}

fn reject(message: impl Into<String>) -> ScriptError {
    ScriptError::Rejected(message.into())
}

/// Operator-attested behavior; MCP tool annotations alone do not qualify.
/// All non-verified tools must use a requires-approval grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingPolicy {
    VerifiedReadOnly,
    RequiresApproval,
}

/// This mapping must originate from trusted registry/configuration, not a
/// generated script. Argument order is explicit: JSON object key order is not.
#[derive(Clone, Debug)]
pub struct BindingSpec {
    pub alias: String,
    pub capability: CapabilityRef,
    pub arguments: Vec<String>,
    pub policy: BindingPolicy,
    pub sensitive_output: bool,
    pub accepts_sensitive: bool,
}

#[derive(Clone)]
struct Binding {
    spec: BindingSpec,
    declaration: CapabilityDeclaration,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ValueExpr {
    Literal { value: Value },
    ResultField { step: usize, path: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub capability: CapabilityRef,
    pub arguments: BTreeMap<String, ValueExpr>,
}

/// Read-only inspection format, not a bearer token. Only a CompiledProgram
/// created by Compiler can be executed; serialized plans aren't executable.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionPlan {
    pub language: &'static str,
    pub steps: Vec<Step>,
    pub output: ValueExpr,
}

pub struct CompiledProgram {
    plan: ActionPlan,
    owner: AppId,
    bindings: Vec<Binding>,
}

impl CompiledProgram {
    pub fn inspect(&self) -> &ActionPlan {
        &self.plan
    }
}

#[derive(Clone)]
struct TypedExpr {
    value: ValueExpr,
    schema: Value,
    sensitive: bool,
}

impl TypedExpr {
    fn field(&self, name: &str) -> Result<Self, ScriptError> {
        let properties = self.schema.get("properties").and_then(Value::as_object)
            .ok_or_else(|| reject("field access requires a declared object output schema"))?;
        let property = properties.get(name)
            .ok_or_else(|| reject(format!("unknown output field {name:?}")))?;
        let required = self.schema.get("required").and_then(Value::as_array);
        if !required.is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some(name))) {
            return Err(reject(format!("output field {name:?} is optional and may be absent")));
        }
        checked_type(property)?;
        let ValueExpr::ResultField { step, path } = &self.value else {
            return Err(reject("field access is only supported on capability results"));
        };
        let mut path = path.clone();
        path.push(name.to_string());
        Ok(Self {
            value: ValueExpr::ResultField { step: *step, path },
            schema: property.clone(),
            sensitive: self.sensitive,
        })
    }
}

fn checked_type(schema: &Value) -> Result<&str, ScriptError> {
    let name = schema.get("type").and_then(Value::as_str)
        .ok_or_else(|| reject("an explicit simple JSON Schema type is required"))?;
    if !matches!(name, "string" | "integer" | "number" | "boolean" | "object" | "array" | "null") {
        return Err(reject(format!("unsupported schema type {name:?}")));
    }
    Ok(name)
}

fn compatible(actual: &Value, expected: &Value) -> Result<bool, ScriptError> {
    let actual = checked_type(actual)?;
    let expected = checked_type(expected)?;
    Ok(actual == expected || (actual == "integer" && expected == "number"))
}

pub struct Compiler {
    owner: AppId,
    bindings: BTreeMap<String, Binding>,
}

impl Compiler {
    pub fn new(kernel: &Kernel, owner: AppId, specs: Vec<BindingSpec>) -> Result<Self, ScriptError> {
        kernel.installed_app(&owner)?;
        if specs.len() > MAX_STEPS {
            return Err(reject("too many configured capability bindings"));
        }
        let mut bindings = BTreeMap::new();
        for spec in specs {
            if !valid_alias(&spec.alias) {
                return Err(reject(format!("invalid capability alias {:?}", spec.alias)));
            }
            let declaration = kernel.capability_declaration(&spec.capability)?.clone();
            if declaration.input_schema.get("type").and_then(Value::as_str) != Some("object") {
                return Err(reject(format!("{} needs an object input schema", spec.alias)));
            }
            let properties = declaration.input_schema.get("properties")
                .and_then(Value::as_object).ok_or_else(|| reject("input properties are required"))?;
            let required = declaration.input_schema.get("required")
                .and_then(Value::as_array).cloned().unwrap_or_default();
            if spec.arguments.len() > MAX_ARGUMENTS ||
                spec.arguments.iter().collect::<BTreeSet<_>>().len() != spec.arguments.len() ||
                spec.arguments.iter().any(|name| !properties.contains_key(name)) ||
                required.iter().any(|name| !spec.arguments.iter().any(|arg| name.as_str() == Some(arg)))
            {
                return Err(reject(format!("invalid positional argument mapping for {}", spec.alias)));
            }
            for name in &spec.arguments {
                checked_type(&properties[name])?;
            }
            if let Some(schema) = &declaration.output_schema {
                checked_type(&Value::Object(schema.clone()))?;
            }
            let alias = spec.alias.clone();
            if bindings.insert(alias.clone(), Binding { spec, declaration }).is_some() {
                return Err(reject(format!("duplicate alias {alias}")));
            }
        }
        Ok(Self { owner, bindings })
    }

    pub fn compile(&self, source: &str) -> Result<CompiledProgram, ScriptError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(reject("source byte budget exceeded"));
        }
        let mut engine = Engine::new();
        engine.set_optimization_level(OptimizationLevel::None);
        engine.set_max_operations(MAX_NODES as u64);
        engine.set_max_variables(MAX_STEPS * 2);
        let ast = engine.compile(source).map_err(|error| reject(format!("syntax: {error}")))?;
        if ast.iter_functions().next().is_some() {
            return Err(reject("functions and closures are forbidden"));
        }
        let mut node_count = 0usize;
        ast.walk(&mut |_| {
            node_count += 1;
            node_count <= MAX_NODES
        });
        if node_count > MAX_NODES {
            return Err(reject("AST node budget exceeded"));
        }
        let mut builder = Builder {
            bindings: &self.bindings,
            locals: BTreeMap::new(),
            steps: Vec::new(),
        };
        let mut result = None;
        for (index, statement) in ast.statements().iter().enumerate() {
            if result.is_some() {
                return Err(reject("return must be the last statement"));
            }
            match statement {
                Stmt::Noop(_) => {}
                Stmt::Var(data, _, _) => {
                    let (ident, expr, _) = &**data;
                    let name = ident.name.to_string();
                    if !valid_identifier(&name) || builder.locals.contains_key(&name) {
                        return Err(reject(format!("invalid or repeated binding {name:?}")));
                    }
                    let value = builder.expression(expr)?;
                    builder.locals.insert(name, value);
                }
                Stmt::Return(Some(expr), flags, _) if !flags.contains(ASTFlags::BREAK) => {
                    result = Some(builder.expression(expr)?);
                }
                Stmt::Expr(expr) => {
                    if !is_call(expr) {
                        return Err(reject("only capability calls can be expression statements"));
                    }
                    builder.expression(expr)?;
                }
                _ => return Err(reject(format!("unsupported statement at index {index}"))),
            }
        }
        let result = result.ok_or_else(|| reject("explicit return is required"))?;
        Ok(CompiledProgram {
            plan: ActionPlan {
                language: LANGUAGE,
                steps: builder.steps,
                output: result.value,
            },
            owner: self.owner.clone(),
            bindings: self.bindings.values().cloned().collect(),
        })
    }
}

fn valid_identifier(name: &str) -> bool {
    !name.is_empty() &&
        name.bytes().next().is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_') &&
        name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_alias(alias: &str) -> bool {
    let mut parts = alias.split('.');
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None)
        if valid_identifier(a) && valid_identifier(b))
}

fn is_call(expr: &Expr) -> bool {
    matches!(expr, Expr::Dot(data, _, _) if
        matches!(&data.rhs, Expr::MethodCall(..) | Expr::FnCall(..)))
}

struct Builder<'a> {
    bindings: &'a BTreeMap<String, Binding>,
    locals: BTreeMap<String, TypedExpr>,
    steps: Vec<Step>,
}

impl Builder<'_> {
    fn expression(&mut self, expr: &Expr) -> Result<TypedExpr, ScriptError> {
        match expr {
            Expr::StringConstant(text, _) => Ok(literal(json!(text.to_string()), "string")),
            Expr::IntegerConstant(number, _) => Ok(literal(json!(number), "integer")),
            Expr::BoolConstant(value, _) => Ok(literal(json!(value), "boolean")),
            Expr::Unit(_) => Ok(literal(Value::Null, "null")),
            Expr::Variable(data, _, _) => self.locals.get(data.1.as_str())
                .cloned().ok_or_else(|| reject(format!("unknown variable {:?}", data.1))),
            Expr::Dot(data, flags, _) if flags.is_empty() => {
                match &data.rhs {
                    Expr::Property(property, _) => {
                        self.expression(&data.lhs)?.field(property.2.as_str())
                    }
                    Expr::MethodCall(call, _) | Expr::FnCall(call, _) => {
                        let Expr::Variable(provider, _, _) = &data.lhs else {
                            return Err(reject("capability calls must use a configured provider alias"));
                        };
                        let alias = format!("{}.{}", provider.1, call.name);
                        self.call(&alias, call)
                    }
                    _ => Err(reject("unsupported dot expression")),
                }
            }
            _ => Err(reject("unsupported expression (no arithmetic, indexing, eval, or dynamic calls)")),
        }
    }

    fn call(&mut self, alias: &str, call: &FnCallExpr) -> Result<TypedExpr, ScriptError> {
        let binding = self.bindings.get(alias)
            .ok_or_else(|| reject(format!("unknown capability alias {alias:?}")))?.clone();
        if self.steps.len() >= MAX_STEPS || call.args.len() != binding.spec.arguments.len() {
            return Err(reject(format!("{alias}: step budget or argument count exceeded")));
        }
        // Arguments can contain nested calls, which are lowered into earlier
        // steps. We never re-evaluate an expression when advancing a session.
        let mut arguments = BTreeMap::new();
        for (name, expression) in binding.spec.arguments.iter().zip(call.args.iter()) {
            let actual = self.expression(expression)?;
            let expected = &binding.declaration.input_schema["properties"][name];
            if !compatible(&actual.schema, expected)? {
                return Err(reject(format!("{alias}.{name}: input type mismatch")));
            }
            if actual.sensitive && !binding.spec.accepts_sensitive {
                return Err(reject(format!("{alias}: sensitive result cannot flow to this capability")));
            }
            arguments.insert(name.clone(), actual.value);
        }
        if self.steps.len() >= MAX_STEPS {
            return Err(reject("step budget exceeded"));
        }
        let step = self.steps.len();
        self.steps.push(Step {
            capability: binding.spec.capability.clone(),
            arguments,
        });
        let schema = binding.declaration.output_schema.as_ref()
            .map(|schema| Value::Object(schema.clone()))
            .unwrap_or(Value::Null);
        Ok(TypedExpr {
            value: ValueExpr::ResultField { step, path: Vec::new() },
            schema,
            sensitive: binding.spec.sensitive_output,
        })
    }
}

fn literal(value: Value, ty: &str) -> TypedExpr {
    TypedExpr { value: ValueExpr::Literal { value }, schema: json!({"type": ty}), sensitive: false }
}

fn resolve(expr: &ValueExpr, outputs: &[Value]) -> Result<Value, ScriptError> {
    match expr {
        ValueExpr::Literal { value } => Ok(value.clone()),
        ValueExpr::ResultField { step, path } => {
            let mut value = outputs.get(*step).ok_or_else(|| ScriptError::Execution(
                format!("step {step} is not completed")
            ))?;
            for field in path {
                value = value.get(field).ok_or_else(|| ScriptError::Execution(
                    format!("step {step} result lacks field {field:?}")
                ))?;
            }
            Ok(value.clone())
        }
    }
}

/// A session holds a single pending kernel token, not a re-playable script
/// continuation. Calling resume() consumes it. No Rhai evaluation occurs.
pub struct Session {
    program: CompiledProgram,
    run_id: RunId,
    cursor: usize,
    results: Vec<Value>,
    pending: Option<PreparedInvocation>,
    ended: bool,
    started: Instant,
    output_bytes: usize,
}

#[derive(Debug, PartialEq)]
pub enum Progress {
    AwaitingAuthorization { step: usize, capability: CapabilityRef },
    StepCompleted { step: usize },
    Completed { result: Value },
    Stopped { outcome: InvocationResult, completed_steps: usize },
}

impl Session {
    pub fn start(kernel: &mut Kernel, program: CompiledProgram, goal: &str)
        -> Result<Self, ScriptError>
    {
        // A plan is no authority: all identities must still exist in the
        // registry at dispatch time, and grants are checked per invocation.
        kernel.installed_app(&program.owner)?;
        for bound in &program.bindings {
            let current = kernel.capability_declaration(&bound.spec.capability)?;
            if current != &bound.declaration {
                return Err(reject("capability declaration changed since compilation"));
            }
        }
        let run_id = kernel.start_run(Initiator::App {
            app_id: program.owner.clone(),
            reason: "restricted action script".into(),
        }, goal)?;
        Ok(Self {
            program, run_id, cursor: 0, results: Vec::new(),
            pending: None, ended: false, started: Instant::now(), output_bytes: 0,
        })
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn prepare_next(&mut self, kernel: &mut Kernel) -> Result<Progress, ScriptError> {
        if self.ended {
            return Err(ScriptError::Execution("run already terminated".into()));
        }
        if self.pending.is_some() {
            let step = &self.program.plan.steps[self.cursor];
            return Ok(Progress::AwaitingAuthorization {
                step: self.cursor, capability: step.capability.clone(),
            });
        }
        if self.started.elapsed() > MAX_RUN_TIME {
            self.finish(kernel, RunTerminalState::Failed)?;
            return Err(ScriptError::Execution("run time budget exceeded".into()));
        }
        if self.cursor == self.program.plan.steps.len() {
            let result = resolve(&self.program.plan.output, &self.results)?;
            if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > MAX_OUTPUT_BYTES) {
                self.finish(kernel, RunTerminalState::Failed)?;
                return Err(ScriptError::Execution("return byte budget exceeded".into()));
            }
            self.finish(kernel, RunTerminalState::Completed)?;
            return Ok(Progress::Completed { result });
        }
        let step = self.program.plan.steps[self.cursor].clone();
        let bound = self.program.bindings.iter().find(|bound|
            bound.spec.capability == step.capability)
            .cloned().ok_or_else(|| reject("step uses a capability without a bound contract"))?;
        // The provider can be upgraded or permissions revoked between calls.
        if kernel.capability_declaration(&step.capability)? != &bound.declaration {
            self.finish(kernel, RunTerminalState::Failed)?;
            return Err(reject("capability declaration changed during execution"));
        }
        if bound.spec.policy == BindingPolicy::RequiresApproval {
            // A silent grant would allow execution without the trusted prompt.
            // Refuse when ANY live covering authorization could bypass chrome.
            let available = kernel.available_capabilities_for(&self.program.owner)?;
            let view = available.iter().find(|view| view.provider_app_id == step.capability.provider
                && view.capability == step.capability.capability);
            if view.is_some_and(|view| view.authorizations.iter().any(|auth|
                auth.condition != GrantCondition::RequiresApproval)) {
                self.finish(kernel, RunTerminalState::Failed)?;
                return Err(reject("unverified capability has a grant that could bypass approval"));
            }
        }
        let mut input = JsonObject::new();
        for (name, expr) in &step.arguments {
            input.insert(name.clone(), resolve(expr, &self.results)?);
        }
        match kernel.prepare_invocation(&self.run_id, &step.capability, InvocationRequest {
            input, data_scope: DataScope::None,
        })? {
            PrepareInvocation::Prepared(prepared) => {
                self.pending = Some(prepared);
                Ok(Progress::AwaitingAuthorization {
                    step: self.cursor, capability: step.capability.clone(),
                })
            }
            PrepareInvocation::Refused(outcome) => self.stop(kernel, outcome),
        }
    }

    /// This is the ONLY continuation path. await_approval calls host-owned
    /// trusted chrome and the kernel binds that answer to this exact input.
    /// A second resume has no pending token and cannot replay the call.
    pub fn resume(&mut self, kernel: &mut Kernel) -> Result<Progress, ScriptError> {
        if self.ended {
            return Err(ScriptError::Execution("run already terminated".into()));
        }
        let pending = self.pending.take()
            .ok_or_else(|| ScriptError::Execution("no prepared step to resume".into()))?;
        let authorization = kernel.authorize_invocation(pending.await_approval())?;
        let outcome = match authorization {
            AuthorizeInvocation::Authorized(authorized) =>
                kernel.finalize_invocation(authorized.execute())?,
            AuthorizeInvocation::Refused(outcome) => outcome,
        };
        match outcome {
            InvocationResult::Completed { result, artifacts: _ } => {
                let size = serde_json::to_vec(&result).map_err(|e|
                    ScriptError::Execution(e.to_string()))?.len();
                self.output_bytes = self.output_bytes.saturating_add(size);
                if size > MAX_OUTPUT_BYTES || self.output_bytes > MAX_TOTAL_OUTPUT_BYTES {
                    self.finish(kernel, RunTerminalState::Failed)?;
                    return Err(ScriptError::Execution(
                        "output budget exceeded after completed effect (no rollback)".into()
                    ));
                }
                let completed = self.cursor;
                self.cursor += 1;
                self.results.push(result);
                Ok(Progress::StepCompleted { step: completed })
            }
            refused_or_failed => self.stop(kernel, refused_or_failed),
        }
    }

    pub fn cancel(mut self, kernel: &mut Kernel) -> Result<(), ScriptError> {
        if !self.ended {
            if let Some(token) = self.pending.take() {
                kernel.abort_prepared_invocation(token)?;
            }
            self.finish(kernel, RunTerminalState::Cancelled)?;
        }
        Ok(())
    }

    fn stop(&mut self, kernel: &mut Kernel, outcome: InvocationResult)
        -> Result<Progress, ScriptError>
    {
        let terminal = match outcome {
            InvocationResult::Refused { .. } => RunTerminalState::Cancelled,
            _ => RunTerminalState::Failed,
        };
        self.finish(kernel, terminal)?;
        Ok(Progress::Stopped { outcome, completed_steps: self.cursor })
    }

    fn finish(&mut self, kernel: &mut Kernel, terminal: RunTerminalState)
        -> Result<(), ScriptError>
    {
        self.ended = true;
        kernel.end_run(&self.run_id, terminal)?;
        Ok(())
    }
}
