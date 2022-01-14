use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Map a role in AWS IAM to Kubernetes groups
#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(group = "aws-auth.controlant.com", version = "v1", kind = "MapRole", namespaced)]
pub struct MapRoleSpec {
  /// ARN of the AWS Role
  pub rolearn: String,
  /// Username inside kube
  pub username: String,
  /// Groups in kube
  pub groups: Vec<String>,
}
