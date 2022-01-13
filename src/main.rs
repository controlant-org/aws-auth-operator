use anyhow::{bail, Context as _};
use futures_util::StreamExt;
use json_patch::{PatchOperation, ReplaceOperation, TestOperation};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
  api::{Api, ListParams, Patch, PatchParams},
  runtime::{
    controller::{self, Context, Controller, ReconcilerAction},
    finalizer,
  },
  Client, CustomResource, CustomResourceExt, Resource,
};
use log::{debug, error, info};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Map a role in AWS IAM to Kubernetes groups
#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(group = "aws-auth.controlant.com", version = "v1", kind = "MapRole", namespaced)]
pub struct MapRoleSpec {
  /// ARN of the AWS Role
  rolearn: String,
  /// Username inside kube
  username: String,
  /// Groups in kube
  groups: Vec<String>,
}

#[derive(Debug, Error)]
enum AppError {
  #[error("Kube error: {0:?}")]
  KubeError(#[from] kube::Error),
  #[error("Yaml decode error: {0:?}")]
  YamlError(#[from] serde_yaml::Error),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  env_logger::init();

  println!("{}", serde_yaml::to_string(&MapRole::crd()).unwrap());

  // try load from env var which Terraform uses
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

  Controller::new(crd, ListParams::default())
    .run(
      |maprole, ctx| {
        debug!("Reconcile for: {:?}", &maprole);

        let client = ctx.get_ref().clone();
        let namespace = maprole.meta().namespace.as_deref().unwrap();
        let mr_api = Api::<MapRole>::namespaced(client.clone(), &namespace);
        let sys_api = Api::<ConfigMap>::namespaced(client.clone(), "kube-system");
        async move {
          finalizer::finalizer(&mr_api, "aws-auth-operator.controlant.com", maprole, |ev| async {
            match ev {
              finalizer::Event::Apply(mr) => apply(mr, &sys_api).await,
              finalizer::Event::Cleanup(mr) => cleanup(mr, &sys_api).await,
            }
          })
          .await
        }
      },
      |_, _| requeue(60),
      Context::new(client),
    )
    .for_each(|res| async move {
      match res {
        Ok(o) => {
          info!("Reconciled {:?}", o);
        }
        Err(controller::Error::ObjectNotFound(or)) => {
          info!("Object not found: {:?}", or);
        }
        Err(e) => {
          error!("Reconcile failed: {:?}", e);
        }
      }
    })
    .await;

  Ok(())
}

async fn apply(mr: MapRole, api: &Api<ConfigMap>) -> Result<ReconcilerAction, AppError> {
  let aws_auth_cm = api.get("aws-auth").await?;
  let cm_maproles_str = aws_auth_cm.data.as_ref().unwrap().get("mapRoles").unwrap();
  let mut cm_maproles: Vec<MapRoleSpec> = serde_yaml::from_str(cm_maproles_str)?;

  if let Some(mut entry) = cm_maproles.iter_mut().find(|e| e.rolearn == mr.spec.rolearn) {
    if (entry.username != mr.spec.username) || (entry.groups != mr.spec.groups) {
      // update existing entry
      entry.username = mr.spec.username;
      entry.groups = mr.spec.groups;
    } else {
      return Ok(requeue(300));
    }
  } else {
    // add new entry
    cm_maproles.push(mr.spec.clone());
  }

  api
    .patch(
      "aws-auth",
      &PatchParams::default(),
      &Patch::<()>::Json(json_patch::Patch(vec![
        PatchOperation::Test(TestOperation {
          path: "/data/mapRoles".to_string(),
          value: cm_maproles_str.clone().into(),
        }),
        PatchOperation::Replace(ReplaceOperation {
          path: "/data/mapRoles".to_string(),
          value: serde_yaml::to_string(&cm_maproles)?.into(),
        }),
      ])),
    )
    .await?;

  Ok(requeue(300))
}

async fn cleanup(mr: MapRole, api: &Api<ConfigMap>) -> Result<ReconcilerAction, AppError> {
  let aws_auth_cm = api.get("aws-auth").await?;
  let cm_maproles_str = aws_auth_cm.data.as_ref().unwrap().get("mapRoles").unwrap();
  let mut cm_maproles: Vec<MapRoleSpec> = serde_yaml::from_str(cm_maproles_str)?;

  if let Some((idx, _)) = cm_maproles
    .iter()
    .enumerate()
    .find(|(_, e)| e.rolearn == mr.spec.rolearn)
  {
    cm_maproles.remove(idx);

    api
      .patch(
        "aws-auth",
        &PatchParams::default(),
        &Patch::<()>::Json(json_patch::Patch(vec![
          PatchOperation::Test(TestOperation {
            path: "/data/mapRoles".to_string(),
            value: cm_maproles_str.clone().into(),
          }),
          PatchOperation::Replace(ReplaceOperation {
            path: "/data/mapRoles".to_string(),
            value: serde_yaml::to_string(&cm_maproles)?.into(),
          }),
        ])),
      )
      .await?;
  }

  Ok(requeue(0))
}

fn requeue(secs: u64) -> ReconcilerAction {
  match secs {
    0 => ReconcilerAction { requeue_after: None },
    t => ReconcilerAction {
      requeue_after: Some(std::time::Duration::from_secs(t)),
    },
  }
}
