use reqwest::Method;
use sigillum_api::request::{
    ConsolidationPlanApproveRequest, ConsolidationPlanExportRequest,
    ConsolidationPlanGenerateRequest, ConsolidationPlanSimulateRequest, PlanEnqueuePlanRequest,
    PlanEnqueueStepRequest,
};
use sigillum_api::response::{
    ConsolidationPlanExportResponse, ConsolidationPlanListResponse,
    ConsolidationPlanMutationResponse, PlanEnqueuePlanResponse, PlanEnqueueStepResponse,
};
use sigillum_api::route_paths as p;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_consolidation_plans(
        &self,
    ) -> Result<ConsolidationPlanListResponse, ClientError> {
        let builder = self.request(Method::GET, p::API_PLANS_CONSOLIDATION);
        self.send(builder).await
    }

    pub async fn generate_consolidation_plan(
        &self,
        request: ConsolidationPlanGenerateRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_CONSOLIDATION_GENERATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn approve_consolidation_plan(
        &self,
        request: ConsolidationPlanApproveRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_CONSOLIDATION_APPROVE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn simulate_consolidation_plan(
        &self,
        request: ConsolidationPlanSimulateRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_CONSOLIDATION_SIMULATE)
            .json(&request);
        self.send(builder).await
    }

    pub async fn export_consolidation_plan(
        &self,
        request: ConsolidationPlanExportRequest,
    ) -> Result<ConsolidationPlanExportResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_CONSOLIDATION_EXPORT)
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_plan_step(
        &self,
        request: PlanEnqueueStepRequest,
    ) -> Result<PlanEnqueueStepResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_ENQUEUE_STEP)
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_plan(
        &self,
        request: PlanEnqueuePlanRequest,
    ) -> Result<PlanEnqueuePlanResponse, ClientError> {
        let builder = self
            .request(Method::POST, p::API_PLANS_ENQUEUE_PLAN)
            .json(&request);
        self.send(builder).await
    }
}
