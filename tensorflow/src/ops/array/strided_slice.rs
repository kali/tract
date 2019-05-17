use ndarray::prelude::*;
use tract_core::internal::*;

pub fn build(pb: &crate::tfpb::node_def::NodeDef) -> TractResult<Box<InferenceOp>> {
    let begin_mask = pb.get_attr_opt_int("begin_mask")?.unwrap_or(0);
    let end_mask = pb.get_attr_opt_int("end_mask")?.unwrap_or(0);
    let shrink_axis_mask = pb.get_attr_opt_int("shrink_axis_mask")?.unwrap_or(0);
    let datum_type = pb.get_attr_datum_type("T")?;
    let base = BaseStridedSlice::new(begin_mask, end_mask, shrink_axis_mask);
    if datum_type == DatumType::I32 {
        Ok(Box::new(StridedSliceD::new(base)))
    } else {
        Ok(boxed_new!(StridedSlice(datum_type)(base)))
    }
}

#[derive(Debug, Default, Clone, new)]
pub struct BaseStridedSlice {
    begin_mask: i64,
    end_mask: i64,
    shrink_axis_mask: i64,
}

#[derive(Debug, Clone)]
struct Dim {
    begin: TDim,
    end: TDim,
    stride: i32,
    shrink: bool,
}

impl Dim {
    fn len(&self) -> TractResult<usize> {
        Ok((((self.stride.abs() as i32 - 1) + (self.end - self.begin).to_integer()?.abs() as i32)
            / self.stride.abs()) as usize)
    }

    fn soft_len(&self) -> TractResult<TDim> {
        if let Ok(len) = (self.end - self.begin).to_integer() {
            Ok((((self.stride.abs() as i32 - 1) + len.abs() as i32) / self.stride.abs()).to_dim())
        } else if self.stride == 1 {
            Ok(self.end - self.begin)
        } else {
            bail!("Streaming dimensions with strides are not supported for now")
        }
    }
}

impl BaseStridedSlice {
    fn must_shrink(&self, ix: usize) -> bool {
        self.shrink_axis_mask & (1 << ix) != 0
    }
    fn ignore_begin(&self, ix: usize) -> bool {
        self.begin_mask & (1 << ix) != 0
    }
    fn ignore_end(&self, ix: usize) -> bool {
        self.end_mask & (1 << ix) != 0
    }
    fn prepare_one_dim(
        &self,
        ix: usize,
        dim: TDim,
        begin: &ArrayView1<TDim>,
        end: &ArrayView1<TDim>,
        strides: &ArrayView1<i32>,
    ) -> Dim {
        // deal with too small dim begin/end/stride for input rank
        if ix >= begin.len() {
            return Dim { begin: 0.to_dim(), end: dim, stride: 1, shrink: false };
        }

        // deal with negative indexing
        fn must_add_to_len(bound: TDim) -> bool {
            if let Some(b) = bound.as_const() {
                b < 0
            } else {
                bound.eval(100_000_000).unwrap() < 0 // FIXME
            }
        }
        let b: TDim = if must_add_to_len(begin[ix]) { dim + begin[ix] } else { begin[ix] };
        let e: TDim = if must_add_to_len(end[ix]) { dim + end[ix] } else { end[ix] };

        // deal with shrinking
        if self.must_shrink(ix) {
            return Dim { begin: b, end: b + 1, stride: 1, shrink: true };
        }

        // deal with begin and end masks
        let s = strides[ix];
        let b = if self.ignore_begin(ix) {
            if s.signum() > 0 {
                0.to_dim()
            } else {
                dim - 1
            }
        } else {
            b
        };
        let e = if self.ignore_end(ix) {
            if s.signum() < 0 {
                -1.to_dim()
            } else {
                dim
            }
        } else {
            e
        };
        Dim { begin: b, end: e, stride: s, shrink: false }
    }

    fn prepare(
        &self,
        input_shape: &[usize],
        begin: Arc<Tensor>,
        end: Arc<Tensor>,
        strides: Arc<Tensor>,
    ) -> TractResult<(Vec<Dim>, Vec<usize>, Vec<usize>)> {
        let casted_begin = begin.cast_to::<TDim>()?;
        let begin = casted_begin.to_array_view::<TDim>()?.into_dimensionality()?;
        let casted_end = end.cast_to::<TDim>()?;
        let end = casted_end.to_array_view::<TDim>()?.into_dimensionality()?;
        let strides = strides.to_array_view::<i32>()?.into_dimensionality()?;
        trace!(
            "StridedSlice {:?} computing shapes: input_shape:{:?} begin:{:?} end:{:?} strides:{:?}",
            self,
            input_shape,
            begin,
            end,
            strides
        );
        let bounds: Vec<Dim> = (0..input_shape.len())
            .map(|ix| self.prepare_one_dim(ix, input_shape[ix].to_dim(), &begin, &end, &strides))
            .collect();
        trace!("StridedSlice bounds {:?}", bounds);
        let mid_shape: Vec<usize> =
            bounds.iter().map(|d| d.len()).collect::<TractResult<Vec<usize>>>()?;
        let end_shape: Vec<usize> = bounds
            .iter()
            .filter(|d| !d.shrink)
            .map(|d| d.len())
            .collect::<TractResult<Vec<usize>>>()?;
        Ok((bounds, mid_shape, end_shape))
    }

    fn eval<T: Copy + Datum>(
        &self,
        mut inputs: TVec<Arc<Tensor>>,
    ) -> TractResult<TVec<Arc<Tensor>>> {
        let (input, begin, end, strides) = args_4!(inputs);
        let (bounds, mid_shape, end_shape) = self.prepare(input.shape(), begin, end, strides)?;
        let input = input.to_array_view::<T>()?;
        let output = Array::from_shape_fn(mid_shape, |coords| {
            let coord: Vec<_> = coords
                .slice()
                .iter()
                .enumerate()
                .map(|(d, i)| {
                    (*i as i32 * bounds[d].stride + bounds[d].begin.to_integer().unwrap() as i32)
                        as usize
                })
                .collect();
            input[&*coord]
        });
        let output = output.into_shape(end_shape)?;
        Ok(tvec![output.into_arc_tensor()])
    }

    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        s: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        check_input_arity(&inputs, 4)?;
        check_output_arity(&outputs, 1)?;
        s.equals(&inputs[0].datum_type, &outputs[0].datum_type)?;
        s.equals(&inputs[1].rank, 1)?;
        s.equals(&inputs[2].rank, 1)?;
        s.equals(&inputs[3].rank, 1)?;
        s.equals_all(wrap!(&inputs[1].shape[0], &inputs[2].shape[0], &inputs[3].shape[0]))?;
        s.given_4(
            &inputs[0].shape,
            &inputs[1].value,
            &inputs[2].value,
            &inputs[3].value,
            move |s, input_shape, begin, end, stride| {
                let casted_begin = begin.cast_to::<TDim>()?;
                let begin = casted_begin.to_array_view::<TDim>()?.into_dimensionality()?;
                let casted_end = end.cast_to::<TDim>()?;
                let end = casted_end.to_array_view::<TDim>()?.into_dimensionality()?;
                let stride = stride.to_array_view::<i32>()?.into_dimensionality()?;
                let mut current_out_dim = 0;
                for (ix, d) in input_shape.iter().enumerate() {
                    if !self.must_shrink(ix) {
                        let preped = self.prepare_one_dim(ix, *d, &begin, &end, &stride);
                        s.equals(&outputs[0].shape[current_out_dim], preped.soft_len()?)?;
                        current_out_dim += 1;
                    }
                }
                s.equals(&outputs[0].rank, current_out_dim as i32)
            },
        )
    }
}

#[derive(Debug, Default, Clone, new)]
pub struct StridedSlice<T: Copy + Datum> {
    base: BaseStridedSlice,
    _phantom: PhantomData<T>,
}

impl<T: Copy + Datum> StatelessOp for StridedSlice<T> {
    fn eval(&self, inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        self.base.eval::<T>(inputs)
    }
}

impl<T: Copy + Datum> Op for StridedSlice<T> {
    fn name(&self) -> Cow<str> {
        "tf.StridedSlice".into()
    }

    fn declutter(
        &self,
        model: &TypedModel,
        node: &TypedNode,
    ) -> TractResult<Option<TypedModelPatch>> {
        let mut inputs = model.node_input_facts(node.id)?;
        let (input, begin, end, strides) = args_4!(inputs);
        if let (Some(ref begin), Some(ref end), Some(ref strides)) =
            (begin.konst.as_ref(), end.konst.as_ref(), strides.konst.as_ref())
        {
            if strides.to_array_view::<i32>()?.iter().any(|&s| s != 1) {
                info!("Failed to unarize StridedSlices because of strides");
                return Ok(None);
            }
            let begin = begin.cast_to::<TDim>()?;
            let begin_view = begin.to_array_view::<TDim>()?.into_dimensionality()?;
            let end = end.cast_to::<TDim>()?;
            let end_view = end.to_array_view::<TDim>()?.into_dimensionality()?;
            let strides = strides.cast_to::<i32>()?;
            let strides_view = strides.to_array_view::<i32>()?.into_dimensionality()?;
            let mut prunes = vec![];
            for ix in 0..input.shape.rank() {
                let dim = self.base.prepare_one_dim(
                    ix,
                    input.shape.dim(ix),
                    &begin_view.view(),
                    &end_view.view(),
                    &strides_view.view(),
                );
                prunes.push((
                    dim.begin.to_integer()? as usize,
                    (input.shape.dim(ix) - dim.end).to_integer()? as usize,
                ));
            }
            let op = ::tract_core::ops::array::Slice::new(prunes);
            return Ok(Some(TypedModelPatch::single_unary_op(model, node, op)?));
        }
        Ok(None)
    }
}

impl<T: Copy + Datum> InferenceRulesOp for StridedSlice<T> {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        solver: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        self.base.rules(solver, inputs, outputs)
    }

    inference_op_as_op!();
}

#[derive(Debug, Default, Clone, new)]
pub struct StridedSliceD {
    base: BaseStridedSlice,
}

impl StatelessOp for StridedSliceD {
    fn eval(&self, inputs: TVec<Arc<Tensor>>) -> TractResult<TVec<Arc<Tensor>>> {
        let dt = inputs[0].datum_type();
        match dt {
            DatumType::TDim => self.base.eval::<TDim>(inputs),
            DatumType::I32 => self.base.eval::<i32>(inputs),
            _ => panic!("StridedSliceD only covering i32 and Dim"),
        }
    }
}

impl Op for StridedSliceD {
    fn name(&self) -> Cow<str> {
        "tf.StridedSliceD".into()
    }
}

impl InferenceRulesOp for StridedSliceD {
    fn rules<'r, 'p: 'r, 's: 'r>(
        &'s self,
        solver: &mut Solver<'r>,
        inputs: &'p [TensorProxy],
        outputs: &'p [TensorProxy],
    ) -> InferenceResult {
        self.base.rules(solver, inputs, outputs)
    }

    inference_op_as_op!();
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::*;
    use ndarray::*;

    fn eval<I, B, E, S>(op: StridedSlice<i32>, input: I, begin: B, end: E, strides: S) -> Tensor
    where
        I: Into<Tensor>,
        B: Into<Tensor>,
        E: Into<Tensor>,
        S: Into<Tensor>,
    {
        op.eval(tvec![
            input.into().into(),
            begin.into().into(),
            end.into().into(),
            strides.into().into(),
        ])
        .unwrap()
        .pop()
        .unwrap()
        .into_tensor()
    }

    // https://www.tensorflow.org/api_docs/python/tf/strided_slice
    #[test]
    fn eval_1() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                arr3(&[[[1, 1, 1], [2, 2, 2]], [[3, 3, 3], [4, 4, 4]], [[5, 5, 5], [6, 6, 6]],]),
                tensor1(&[1, 0, 0]),
                tensor1(&[2, 1, 3]),
                tensor1(&[1, 1, 1])
            ),
            Tensor::from(arr3(&[[[3, 3, 3]]])),
        );
    }

    #[test]
    fn eval_2() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                arr3(&[[[1, 1, 1], [2, 2, 2]], [[3, 3, 3], [4, 4, 4]], [[5, 5, 5], [6, 6, 6]],]),
                tensor1(&[1, 0, 0]),
                tensor1(&[2, 2, 3]),
                tensor1(&[1, 1, 1])
            ),
            Tensor::from(arr3(&[[[3, 3, 3], [4, 4, 4]]])),
        );
    }

    #[test]
    fn eval_3() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                arr3(&[[[1, 1, 1], [2, 2, 2]], [[3, 3, 3], [4, 4, 4]], [[5, 5, 5], [6, 6, 6]],]),
                tensor1(&[1, -1, 0]),
                tensor1(&[2, -3, 3]),
                tensor1(&[1, -1, 1])
            ),
            Tensor::from(arr3(&[[[4, 4, 4], [3, 3, 3]]])),
        );
    }

    #[test]
    fn eval_4() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                tensor3(&[[[1, 1, 1], [2, 2, 2]], [[3, 3, 3], [4, 4, 4]], [[5, 5, 5], [6, 6, 6]],]),
                tensor1(&[1, 0, 0]),
                tensor1(&[2, 2, 4]),
                tensor1(&[1, 1, 2])
            ),
            tensor3(&[[[3, 3], [4, 4]]]),
        );
    }

    #[test]
    fn eval_5() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                tensor1(&[0, 0]),
                tensor1(&[0]),
                tensor1(&[-1]),
                tensor1(&[1])
            ),
            tensor1(&[0])
        )
    }

    #[test]
    fn eval_6() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                tensor2(&[[1, 0, 0, 0], [3, 0, 0, 0], [0, 0, 0, 0]]),
                tensor1(&[-3, -4]),
                tensor1(&[-1, -1]),
                tensor1(&[1, 2])
            ),
            tensor2(&[[1, 0], [3, 0]])
        )
    }

    #[test]
    fn eval_7() {
        assert_eq!(
            eval(
                StridedSlice::default(),
                tensor2(&[[0, 6], [0, 0]]),
                tensor1(&[0]),
                tensor1(&[2]),
                tensor1(&[1])
            ),
            tensor2(&[[0, 6], [0, 0]])
        )
    }

    #[test]
    fn eval_begin_mask_1() {
        let mut op = StridedSlice::default();
        op.base.begin_mask = 1;
        assert_eq!(
            eval(op, tensor1(&[0, 1]), tensor1(&[1]), tensor1(&[1]), tensor1(&[1])),
            Tensor::from(tensor1(&[0]))
        )
    }

    #[test]
    fn eval_shrink_1() {
        let mut op = StridedSlice::default();
        op.base.shrink_axis_mask = 1;
        assert_eq!(
            eval(op, arr2(&[[0]]), tensor1(&[0, 0]), tensor1(&[0, 0]), tensor1(&[1, 1])),
            tensor1::<i32>(&[])
        )
    }

    #[test]
    fn eval_shrink_to_scalar() {
        let mut op = StridedSlice::default();
        op.base.shrink_axis_mask = 1;
        assert_eq!(
            eval(op, tensor1(&[0]), tensor1(&[0]), tensor1(&[0]), tensor1(&[1])),
            tensor0::<i32>(0)
        )
    }

    #[test]
    fn inference_1() {
        let op = StridedSlice::<f32>::new(BaseStridedSlice::new(5, 7, 0));
        let input = TensorFact::default().with_datum_type(DatumType::F32);
        let begin = TensorFact::from(tensor1(&[0i32, 2, 0]));
        let end = TensorFact::from(tensor1(&[0i32, 0, 0]));
        let strides = TensorFact::from(tensor1(&[1i32, 1, 1]));
        let any = TensorFact::default();

        let (input_facts, output_facts) =
            op.infer_facts(tvec![&input, &begin, &end, &strides], tvec![&any]).unwrap();
        assert_eq!(
            input_facts,
            tvec![
                TensorFact::default().with_datum_type(DatumType::F32).with_shape(shapefact![..]),
                begin,
                end,
                strides,
            ]
        );
        assert_eq!(
            output_facts,
            tvec![TensorFact::default().with_datum_type(DatumType::F32).with_shape(shapefact![..]),]
        );
    }

    #[test]
    fn inference_2() {
        let op = StridedSlice::<f32>::new(BaseStridedSlice::new(1, 1, 2));
        let input = TensorFact::default().with_datum_type(DatumType::F32);
        let begin = TensorFact::from(tensor1(&[0i32, 0]));
        let end = TensorFact::from(tensor1(&[0i32, 1]));
        let strides = TensorFact::from(tensor1(&[1i32, 1]));
        let any = TensorFact::default();

        let (input_facts, output_facts) =
            op.infer_facts(tvec![&input, &begin, &end, &strides], tvec![&any]).unwrap();
        assert_eq!(
            input_facts,
            tvec![
                TensorFact::default().with_datum_type(DatumType::F32).with_shape(shapefact![..]),
                begin,
                end,
                strides,
            ]
        );
        assert_eq!(
            output_facts,
            tvec![TensorFact::default().with_datum_type(DatumType::F32).with_shape(shapefact![..]),]
        );
    }

    #[test]
    fn inference_3() {
        let op = StridedSlice::<f32>::new(BaseStridedSlice::new(5, 7, 0));
        let input = TensorFact::dt_shape(DatumType::F32, shapefact!(1, (TDim::stream() - 2), 16));
        let begin = TensorFact::from(tensor1(&[0i32, 2, 0]));
        let end = TensorFact::from(tensor1(&[0i32, 0, 0]));
        let strides = TensorFact::from(tensor1(&[1i32, 1, 1]));
        let any = TensorFact::default();

        let (_, output_facts) =
            op.infer_facts(tvec![&input, &begin, &end, &strides], tvec![&any]).unwrap();

        assert_eq!(
            output_facts,
            tvec![TensorFact::dt_shape(DatumType::F32, shapefact!(1, (TDim::stream() - 4), 16))]
        );
    }

    #[test]
    fn inference_4() {
        let op = StridedSlice::<f32>::new(BaseStridedSlice::new(5, 7, 0));
        let input = TensorFact::dt_shape(DatumType::F32, shapefact!(1, (TDim::stream() - 2), 16));
        let begin = TensorFact::from(tensor1(&[0i32, 2, 0]));
        let end = TensorFact::from(tensor1(&[0i32, 0, 0]));
        let strides = TensorFact::from(tensor1(&[1i32, 1, 1]));
        let any = TensorFact::default();

        let (_, output_facts) =
            op.infer_facts(tvec![&input, &begin, &end, &strides], tvec![&any]).unwrap();

        assert_eq!(
            output_facts,
            tvec![TensorFact::dt_shape(DatumType::F32, shapefact!(1, (TDim::stream() - 4), 16))]
        );
    }
}
