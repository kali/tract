use ndarray::prelude::*;
use tract_core::internal::*;

pub fn build(pb: &crate::tfpb::node_def::NodeDef) -> TractResult<Box<InferenceOp>> {
    let n = pb.get_attr_int("N")?;
    let t = pb.get_attr_datum_type("T")?;
    let tidx = pb.get_attr_datum_type("Tidx")?;
    Ok(boxed_new!(ConcatV2(t)(n, tidx)))
}

#[derive(Debug, Clone, new)]
pub struct ConcatV2<T: Copy + Datum> {
    n: usize,
    tidx: DatumType,
    t: PhantomData<T>,
}

impl<T: Copy + Datum> StatelessOp for ConcatV2<T> {
    fn eval(&self, mut inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        let axis: i32 = *inputs.pop().unwrap().to_scalar::<i32>()?;
        let mats: TractResult<Vec<ArrayViewD<T>>> =
            inputs.iter().map(|mat| mat.to_array_view()).collect();
        let result = ::ndarray::stack(Axis(axis as usize), &*mats?)?;
        Ok(tvec![result.into_arc_tensor()])
    }
}

impl<T: Copy + Datum> Op for ConcatV2<T> {
    fn name(&self) -> Cow<str> {
        "tf.ConvatV2".into()
    }
}

impl<T: Copy + Datum> InferenceRulesOp for ConcatV2<T> {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        s: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        check_input_arity(&inputs, self.n + 1)?;
        check_output_arity(&outputs, 1)?;
        s.equals_all((0..self.n).map(|i| (&inputs[i].datum_type).bex()).collect())?;
        s.equals(&outputs[0].datum_type, &inputs[0].datum_type)?;
        s.equals(&inputs[self.n].datum_type, DatumType::I32)?;
        s.equals_all((0..self.n).map(|i| (&inputs[i].rank).bex()).collect())?;
        s.equals(&inputs[self.n].rank, 0)?;
        s.equals(&outputs[0].rank, &inputs[0].rank)?;
        s.given(&inputs[self.n].value, move |s, axis| {
            let axis = *axis.to_scalar::<i32>()? as usize;
            trace!("axis for Concatv2: {}", axis);
            for d in 0..axis {
                s.equals_all((0..self.n).map(|i| (&inputs[i].shape[d]).bex()).collect())?;
            }
            for d in 0..axis {
                s.equals(&inputs[0].shape[d], &outputs[0].shape[d])?;
            }
            s.given(&inputs[0].rank, move |s, rank| {
                trace!("Given rank {}", rank);
                for d in (axis + 1)..(rank as usize) {
                    s.equals(&inputs[0].shape[d], &outputs[0].shape[d])?;
                }
                for d in (axis + 1)..(rank as usize) {
                    s.equals_all((0..self.n).map(|i| (&inputs[i].shape[d]).bex()).collect())?;
                }
                Ok(())
            })?;

            let mut concat_dim = -1 * outputs[0].shape[axis].bex();
            for i in 0..self.n {
                concat_dim = concat_dim + inputs[i].shape[axis].bex();
            }
            s.equals_zero(concat_dim)
        })
    }

    inference_op_as_op!();
}
