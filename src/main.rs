use anyhow::{bail, Context as _};
use k8s_openapi::{
  api::core::v1::ConfigMap,
  apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference},
};
use kube::{
  api::{Api, ListParams, Patch, PatchParams},
  error::ErrorResponse,
  runtime::{
    controller::{Context, Controller, ReconcilerAction},
    finalizer::{finalizer, Event},
  },
  Client, CustomResource,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Map a role in AWS IAM to Kubernetes groups
#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(group = "aws-auth.controlant.com", version = "v1", kind = "MapRole", namespaced)]
pub struct MapRoleSpec {
  /// ARN of the AWS Role
  role_arn: String,
  /// Username inside kube
  username: String,
  /// Groups in kube
  groups: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  env_logger::init();

  let client = match Client::try_default().await {
    Ok(c) => c,
    _ => {
      use kube::config::KubeConfigOptions;
      use std::convert::TryFrom;

      match std::env::var("KUBE_CTX") {
        Ok(ctx) => Client::try_from(
          kube::Config::from_kubeconfig(&KubeConfigOptions {
            context: Some(ctx),
            ..KubeConfigOptions::default()
          })
          .await?,
        )
        .context("Failed to load KUBE_CTX context")?,

        _ => bail!("Failed to create client"),
      }
    }
  };

  // MAYBE: apply CRD

  let crd = Api::<MapRole>::all(client.clone());

  //   reconcile_all_on

  Ok(())
}
