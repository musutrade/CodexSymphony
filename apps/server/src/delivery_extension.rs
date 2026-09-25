//! Delivery invocation boundary. Concrete targets, credentials and observations
//! belong to the selected adapter; the caller retains the existing action ledger.
use crate::{
    controlled_contract::{Call, ControlledConfig, Operation, Registration},
    extension_contract::{FrozenConfig, ProtocolError},
};
use serde::{Deserialize, Serialize};
use std::future::Future;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// An opaque, reviewed target reference is resolved only by the adapter. It is
/// not a URL, command line or credential supplied by a candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub call: Call,
    pub target_ref: String,
    pub operation_ref: String,
}

pub struct Approval<'a> {
    pub frozen: &'a FrozenConfig,
    pub controlled: &'a ControlledConfig,
    pub installed: &'a [Registration],
    pub target_ref: &'a str,
    pub operation_ref: &'a str,
}

impl Request {
    pub fn check(&self, approval: &Approval<'_>) -> std::result::Result<(), ProtocolError> {
        self.call
            .validate(approval.frozen, approval.controlled, approval.installed)?;
        if self.target_ref.is_empty()
            || self.operation_ref.is_empty()
            || self.target_ref != approval.target_ref
            || self.operation_ref != approval.operation_ref
        {
            return Err(ProtocolError::IdentityMismatch("delivery target/operation"));
        }
        if !matches!(
            self.call.operation,
            Operation::CapabilityCheck
                | Operation::Submit
                | Operation::Observe
                | Operation::Reconcile
        ) {
            return Err(ProtocolError::UnsupportedCapability("delivery operation"));
        }
        Ok(())
    }
}

/// Implementations are selected from deployment-owned registrations, never
/// loaded from the candidate. The request type is the adapter's frozen target,
/// not an arbitrary shell command or API request.
pub trait Adapter {
    type Input: PartialEq;
    type Facts;
    fn capability_check(
        &mut self,
        request: &Self::Input,
    ) -> impl Future<Output = Result<Reply<Self::Input, Self::Facts>>>;
    fn submit(
        &mut self,
        request: &Self::Input,
    ) -> impl Future<Output = Result<Reply<Self::Input, Self::Facts>>>;
    fn observe(
        &mut self,
        request: &Self::Input,
    ) -> impl Future<Output = Result<Reply<Self::Input, Self::Facts>>>;
    fn reconcile(
        &mut self,
        request: &Self::Input,
    ) -> impl Future<Output = Result<Reply<Self::Input, Self::Facts>>>;
}

pub struct Reply<Input, Facts> {
    pub request: Input,
    pub facts: Facts,
}

/// The coordinator implements these using its existing admission and action
/// ledger. Unknown sends cannot acquire a new intent until reconciled.
pub trait Control<Input, Facts> {
    fn check(&self, operation: &Operation, request: &Input) -> Result<()>;
    fn admit(&mut self, request: &Input) -> impl Future<Output = Result<()>>;
    fn before_deliver(&mut self, request: &Input) -> impl Future<Output = Result<()>>;
    fn begin(&mut self, request: &Input) -> impl Future<Output = Result<bool>>;
    fn retain(
        &mut self,
        request: &Input,
        reply: &Result<Reply<Input, Facts>>,
    ) -> impl Future<Output = Result<()>>;
    fn verify(
        &mut self,
        request: &Input,
        reply: &Reply<Input, Facts>,
    ) -> impl Future<Output = Result<()>>;
    fn post_delivery_validate(
        &mut self,
        request: &Input,
        reply: &Reply<Input, Facts>,
    ) -> impl Future<Output = Result<()>>;
}

pub async fn invoke<A: Adapter>(
    adapter: &mut A,
    control: &mut impl Control<A::Input, A::Facts>,
    operation: Operation,
    request: &A::Input,
) -> Result<Option<Reply<A::Input, A::Facts>>> {
    control.check(&operation, request)?;
    if operation == Operation::Submit && !prepare_submit(control, request).await? {
        return Ok(None);
    }
    let result = dispatch(adapter, &operation, request).await;
    control.retain(request, &result).await?;
    let reply = result?;
    if reply.request != *request {
        return Err("delivery result target or operation differs".into());
    }
    control.verify(request, &reply).await?;
    control.post_delivery_validate(request, &reply).await?;
    Ok(Some(reply))
}

async fn prepare_submit<Input, Facts>(
    control: &mut impl Control<Input, Facts>,
    request: &Input,
) -> Result<bool> {
    control.admit(request).await?;
    control.before_deliver(request).await?;
    control.admit(request).await?;
    control.begin(request).await
}

async fn dispatch<A: Adapter>(
    adapter: &mut A,
    operation: &Operation,
    request: &A::Input,
) -> Result<Reply<A::Input, A::Facts>> {
    match operation {
        Operation::CapabilityCheck => adapter.capability_check(request).await,
        Operation::Submit => adapter.submit(request).await,
        Operation::Observe => adapter.observe(request).await,
        Operation::Reconcile => adapter.reconcile(request).await,
        _ => Err("unsupported delivery operation".into()),
    }
}
