use anyhow::{bail, Context as _};
use futures_util::StreamExt;
use json_patch::{PatchOperation, ReplaceOperation, TestOperation};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
  api::{Api, ListParams, Patch, PatchParams},
  runtime::{
    controller::{self, Action, Controller},
    finalizer,
  },
  Client, Resource,
};
use log::{debug, error, info};
use std::{sync::Arc, time::Duration};
use thiserror::Error;

use operator::{MapRole, MapRoleSpec};

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

  let crd = Api::<MapRole>::all(client.clone());

  Controller::new(crd, ListParams::default())
    .run(
      |maprole, ctx| {
        debug!("Reconcile for: {:?}", &maprole);

        let client = ctx.as_ref();
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
      |_, _| Action::requeue(Duration::from_secs(60)),
      Arc::new(client),
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

async fn apply(mr: Arc<MapRole>, api: &Api<ConfigMap>) -> Result<Action, AppError> {
  let aws_auth_cm = api.get("aws-auth").await?;
  let cm_maproles_str = aws_auth_cm.data.as_ref().unwrap().get("mapRoles").unwrap();
  let mut cm_maproles: Vec<MapRoleSpec> = serde_yaml::from_str(cm_maproles_str)?;

  if let Some(mut entry) = cm_maproles.iter_mut().find(|e| e.rolearn == mr.spec.rolearn) {
    if (entry.username != mr.spec.username) || (entry.groups != mr.spec.groups) {
      // update existing entry
      entry.username = mr.spec.username.clone();
      entry.groups = mr.spec.groups.clone();
    } else {
      return Ok(Action::requeue(Duration::from_secs(60)));
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

  Ok(Action::requeue(Duration::from_secs(300)))
}

async fn cleanup(mr: Arc<MapRole>, api: &Api<ConfigMap>) -> Result<Action, AppError> {
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

  Ok(Action::await_change())
}
