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

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    pub async fn list_consolidation_plans(
        &self,
    ) -> Result<ConsolidationPlanListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/plans/consolidation");
        self.send(builder).await
    }

    pub async fn generate_consolidation_plan(
        &self,
        request: ConsolidationPlanGenerateRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/consolidation/generate")
            .json(&request);
        self.send(builder).await
    }

    pub async fn approve_consolidation_plan(
        &self,
        request: ConsolidationPlanApproveRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/consolidation/approve")
            .json(&request);
        self.send(builder).await
    }

    pub async fn simulate_consolidation_plan(
        &self,
        request: ConsolidationPlanSimulateRequest,
    ) -> Result<ConsolidationPlanMutationResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/consolidation/simulate")
            .json(&request);
        self.send(builder).await
    }

    pub async fn export_consolidation_plan(
        &self,
        request: ConsolidationPlanExportRequest,
    ) -> Result<ConsolidationPlanExportResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/consolidation/export")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_plan_step(
        &self,
        request: PlanEnqueueStepRequest,
    ) -> Result<PlanEnqueueStepResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/enqueue-step")
            .json(&request);
        self.send(builder).await
    }

    pub async fn enqueue_plan(
        &self,
        request: PlanEnqueuePlanRequest,
    ) -> Result<PlanEnqueuePlanResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/plans/enqueue-plan")
            .json(&request);
        self.send(builder).await
    }
}
