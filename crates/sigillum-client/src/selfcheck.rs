use reqwest::Method;
use sigillum_api::request::SelfCheckRunRequest;
use sigillum_api::response::SelfCheckRunResponse;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    /// Run the daemon's operator self-check.
    ///
    /// An empty `domains` list (the default) runs every check domain; the
    /// daemon performs live provider probes, so allow a few seconds per
    /// configured provider.
    pub async fn run_self_check(
        &self,
        request: SelfCheckRunRequest,
    ) -> Result<SelfCheckRunResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/selfcheck/run")
            .json(&request);
        self.send(builder).await
    }
}
