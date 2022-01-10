pub trait Finalizer {
  fn apply(&self);
  fn delete(&self);
}
