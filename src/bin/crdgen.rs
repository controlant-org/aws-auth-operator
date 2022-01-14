use kube::CustomResourceExt;

fn main() {
  println!("{}", serde_yaml::to_string(&operator::MapRole::crd()).unwrap());
}
