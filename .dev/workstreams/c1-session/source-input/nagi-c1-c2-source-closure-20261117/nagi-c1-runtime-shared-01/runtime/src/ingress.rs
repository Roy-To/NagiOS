//! Private typed ingress over the real Identity observation and SVC client.
//! No wire codec, peer authenticator or inferred authority is introduced.

pub(crate) mod account_binding;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use consumer::session_binding::{AdmissionFailure, SessionBinding};
use identity::{CurrentIdentityResolver, TrustedCallerContext};
use svc::{CancellationToken, RequestEnvelope, ResponseEnvelope, ServiceClient, ServiceFuture};

struct RuntimeIngress<'runtime> {
    binding: SessionBinding<'runtime>,
    client: &'runtime ServiceClient,
}

impl<'runtime> RuntimeIngress<'runtime> {
    // The trusted runtime must associate this client/transport with this caller.
    // ServiceClient cannot expose/attest that association through its public API.
    // Neither PrincipalId nor request data is accepted as evidence for it here.
    fn supply(
        resolver: &mut impl CurrentIdentityResolver,
        caller: &'runtime dyn TrustedCallerContext,
        client: &'runtime ServiceClient,
        cancellation: CancellationToken,
    ) -> Result<Self, AdmissionFailure> {
        let binding = SessionBinding::resolve(resolver, caller, cancellation)?;
        Ok(Self { binding, client })
    }

    fn call<'call>(
        &'call self,
        request: RequestEnvelope,
    ) -> Result<IngressCall<'call, 'runtime>, AdmissionFailure> {
        let inner = self.binding.call(self.client, request)?;
        Ok(IngressCall {
            binding: &self.binding,
            inner: Some(inner),
            finished: false,
        })
    }

    // Local cancellation, not Identity logout or side-effect rollback.
    fn revoke(&self) {
        self.binding.cancel();
    }
}

// The borrowed trusted-caller trait has no Sync requirement. This wrapper is a
// local Future, not a new Send SVC transport future. It must not outlive ingress,
// caller or client. The issuing service remains owned by the runtime separately.
struct IngressCall<'call, 'runtime> {
    binding: &'call SessionBinding<'runtime>,
    inner: Option<ServiceFuture<'call>>,
    finished: bool,
}

impl Future for IngressCall<'_, '_> {
    type Output = Result<ResponseEnvelope, AdmissionFailure>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.finished, "ingress future polled after completion");
        if let Err(reason) = this.binding.check() {
            this.finished = true;
            this.inner = None;
            return Poll::Ready(Err(reason));
        }
        let result = this.inner.as_mut().unwrap().as_mut().poll(context);
        // The real transport may authorize and enter its provider in one poll.
        // This detects a change during that poll but cannot undo provider entry.
        if let Err(reason) = this.binding.check() {
            this.finished = true;
            this.inner = None;
            return Poll::Ready(Err(reason));
        }
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(response) => {
                this.finished = true;
                this.inner = None;
                Poll::Ready(Ok(response))
            }
        }
    }
}

// Dropping this wrapper drops only the local transport/provider future. Issuer
// revocation does not register a wake with SVC; runtime cancellation does when a
// cooperative provider awaits the token. Independently spawned work may remain.

#[cfg(test)]
mod tests;
