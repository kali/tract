use std::cell;
use {Matrix, Result};
use super::Op;

#[derive(Debug)]
pub struct Identity {
}

impl Identity {
    pub fn build(_: &::tfpb::node_def::NodeDef) -> Result<Identity> {
        Ok(Identity {})
    }
}

impl Op for Identity {
    fn eval(&self, inputs: Vec<Matrix>) -> Result<Vec<Matrix>> {
        Ok(inputs)
    }
}

pub struct Placeholder {
    value: cell::Cell<Option<Matrix>>,
}

impl Placeholder {
    pub fn build(_pb: &::tfpb::node_def::NodeDef) -> Result<Placeholder> {
        Ok(Placeholder { value: None.into() })
    }
}

impl Placeholder {
    pub fn set(&self, v: Matrix) {
        self.value.set(Some(v))
    }
}

impl Op for Placeholder {
    fn eval(&self, _inputs: Vec<Matrix>) -> Result<Vec<Matrix>> {
        unsafe { Ok(vec![(*self.value.as_ptr()).as_ref().unwrap().clone()]) }
    }
}

impl ::std::fmt::Debug for Placeholder {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "Placeholder")
    }
}

#[derive(Debug)]
pub struct Const {
    value: Matrix,
}

impl Const {
    pub fn build(pb: &::tfpb::node_def::NodeDef) -> Result<Const> {
        let tensor = pb.get_attr().get("value").unwrap().get_tensor();
        Ok(Const { value: Matrix::from_pb(&tensor)? })
    }
}

impl Op for Const {
    fn eval(&self, _inputs: Vec<Matrix>) -> Result<Vec<Matrix>> {
        Ok(vec![self.value.clone()])
    }
}

