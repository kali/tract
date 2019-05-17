use crate::internal::*;
use ndarray::prelude::*;

#[derive(Debug, Clone, new, Default)]
pub struct GlobalAvgPool {
    //    data_is_nhwc: bool, // default is nchw (onnx)
}

impl GlobalAvgPool {
    fn eval_t<D: Datum + ::num_traits::Float + ::num_traits::FromPrimitive>(
        &self,
        input: Arc<Tensor>,
    ) -> TractResult<TVec<Arc<Tensor>>> {
        let array = input.to_array_view::<D>()?;
        let n = array.shape()[0];
        let c = array.shape()[1];
        let mut final_shape = array.shape().to_vec();
        for dim in final_shape[2..].iter_mut() {
            *dim = 1;
        }
        let divisor = array.len() / (n * c);
        let result: Tensor = array
            .into_shape(((n * c), divisor))?
            .sum_axis(Axis(1))
            .map(|x| *x / D::from_usize(divisor).unwrap())
            .into_shape(final_shape)?
            .into();
        Ok(tvec!(result.into()))
    }
}

impl Op for GlobalAvgPool {
    fn name(&self) -> Cow<str> {
        "GlobalAvgPool".into()
    }
}

impl StatelessOp for GlobalAvgPool {
    fn eval(&self, mut inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        let input = args_1!(inputs);
        dispatch_floatlike!(Self::eval_t(input.datum_type())(self, input))
    }
}

impl InferenceRulesOp for GlobalAvgPool {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        solver: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        rules(solver, inputs, outputs)
    }

    inference_op_as_op!();
}

#[derive(Debug, Clone, new, Default)]
pub struct GlobalLpPool {
    p: usize, //    data_is_nhwc: bool, // default is nchw (onnx)
}

impl GlobalLpPool {
    fn eval_t<D: Datum + ::num_traits::Float>(
        &self,
        input: Arc<Tensor>,
    ) -> TractResult<TVec<Arc<Tensor>>> {
        let array = input.to_array_view::<D>()?;
        let n = array.shape()[0];
        let c = array.shape()[1];
        let mut final_shape = array.shape().to_vec();
        for dim in final_shape[2..].iter_mut() {
            *dim = 1;
        }
        let divisor = array.len() / (n * c);
        let input = array.into_shape(((n * c), divisor))?;
        let divisor = D::from(divisor).unwrap();
        let result = if self.p == 1 {
            input.fold_axis(Axis(1), D::zero(), |&a, &b| a + b.abs()).map(|a| *a / divisor)
        } else if self.p == 2 {
            input.fold_axis(Axis(1), D::zero(), |&a, &b| a + b * b).map(|a| a.sqrt() / divisor)
        } else {
            input
                .fold_axis(Axis(1), D::zero(), |&a, &b| a + b.abs().powi(self.p as i32))
                .map(|a| a.powf(D::from(self.p).unwrap().recip()) / divisor)
        };
        Ok(tvec!(result.into_shape(final_shape)?.into_arc_tensor()))
    }
}

impl Op for GlobalLpPool {
    fn name(&self) -> Cow<str> {
        "GlobalLpPool".into()
    }
}

impl StatelessOp for GlobalLpPool {
    fn eval(&self, mut inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        let input = args_1!(inputs);
        dispatch_floatlike!(Self::eval_t(input.datum_type())(self, input))
    }
}

impl InferenceRulesOp for GlobalLpPool {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        solver: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        rules(solver, inputs, outputs)
    }

    inference_op_as_op!();
}

#[derive(Debug, Clone, new, Default)]
pub struct GlobalMaxPool {
    //    data_is_nhwc: bool, // default is nchw (onnx)
}

impl GlobalMaxPool {
    fn eval_t<D: Datum + ::num_traits::Float>(
        &self,
        input: Arc<Tensor>,
    ) -> TractResult<TVec<Arc<Tensor>>> {
        let array = input.to_array_view::<D>()?;
        let n = array.shape()[0];
        let c = array.shape()[1];
        let mut final_shape = array.shape().to_vec();
        for dim in final_shape[2..].iter_mut() {
            *dim = 1;
        }
        let divisor = array.len() / (n * c);
        let result: Tensor = array
            .into_shape(((n * c), divisor))?
            .fold_axis(Axis(1), D::min_value(), |a, b| a.max(*b))
            .into_shape(final_shape)?
            .into();
        Ok(tvec!(result.into()))
    }
}

impl Op for GlobalMaxPool {
    fn name(&self) -> Cow<str> {
        "GlobalMaxPool".into()
    }
}

impl StatelessOp for GlobalMaxPool {
    fn eval(&self, mut inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        let input = args_1!(inputs);
        dispatch_floatlike!(Self::eval_t(input.datum_type())(self, input))
    }
}

impl InferenceRulesOp for GlobalMaxPool {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        solver: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        rules(solver, inputs, outputs)
    }

    inference_op_as_op!();
}

fn rules<'r, 'p: 'r, 's: 'r>(
    s: &mut Solver<'r>,
    inputs: &'p [TensorProxy],
    outputs: &'p [TensorProxy],
) -> InferenceResult {
    check_output_arity(&outputs, 1)?;
    s.equals(&outputs[0].datum_type, &inputs[0].datum_type)?;
    s.equals(&outputs[0].rank, &inputs[0].rank)?;
    s.equals(&outputs[0].shape[0], &inputs[0].shape[0])?;
    s.equals(&outputs[0].shape[1], &inputs[0].shape[1])?;
    s.given(&inputs[0].rank, move |s, rank| {
        for i in 2..rank {
            s.equals(&outputs[0].shape[i as usize], TDim::from(1))?;
        }
        Ok(())
    })
}
