use {Matrix, Result};
use super::{ Input, Op };
use ndarray::prelude::*;

#[derive(Debug)]
pub enum DataFormat {
    NHWC,
}

#[derive(Debug, PartialEq)]
pub enum Padding {
    Valid,
    Same,
}

pub struct ImageWrapper<'a, T: 'a>(ArrayView3<'a, T>);

impl<'a, T> ImageWrapper<'a, T> {
    pub fn height(&self) -> usize {
        self.0.shape()[0]
    }
    pub fn width(&self) -> usize {
        self.0.shape()[1]
    }
    pub fn depth(&self) -> usize {
        self.0.shape()[2]
    }
}

pub struct BatchImageWrapper<'a, T: 'a>(ArrayView4<'a, T>);

impl<'a, T> BatchImageWrapper<'a, T> {
    pub fn count(&self) -> usize {
        self.0.shape()[0]
    }
    pub fn height(&self) -> usize {
        self.0.shape()[1]
    }
    pub fn h(&self) -> usize {
        self.0.shape()[1]
    }
    pub fn width(&self) -> usize {
        self.0.shape()[2]
    }
    pub fn w(&self) -> usize {
        self.0.shape()[2]
    }
    pub fn depth(&self) -> usize {
        self.0.shape()[3]
    }
    pub fn d(&self) -> usize {
        self.0.shape()[3]
    }
}

#[derive(Debug)]
pub struct LocalPatch {
    pub _data_format: DataFormat,
    pub padding: Padding,
    pub strides: Vec<usize>,
}

impl LocalPatch {
    pub fn build(pb: &::tfpb::node_def::NodeDef) -> Result<LocalPatch> {
        if let Some(data_format) = pb.get_attr().get("data_format") {
            if data_format.get_s() == b"NCHW" {
                Err("NCHW data_format not implemented")?
            }
        }
        let strides = pb.get_attr()
            .get("strides")
            .ok_or("expect strides in Conv2D args")?
            .get_list()
            .get_i()
            .iter()
            .map(|a| *a as usize)
            .collect();
        let padding = pb.get_attr().get("padding").ok_or(
            "expect padding in Conv2D args",
        )?;
        let padding = match padding.get_s() {
            b"VALID" => Padding::Valid,
            b"SAME" => Padding::Same,
            s => {
                Err(format!(
                    "unsupported Padding {}",
                    String::from_utf8_lossy(s)
                ))?
            }
        };
        Ok(LocalPatch {
            _data_format: DataFormat::NHWC,
            padding,
            strides,
        })
    }

    fn adjusted_dim(
        &self,
        in_rows: usize,
        in_cols: usize,
        (filter_rows, filter_cols): (usize, usize),
    ) -> (usize, usize) {
        let stride = self.strides[1];
        match self.padding {
            Padding::Same => (
                (in_rows as f32 / stride as f32).ceil() as usize,
                (in_cols as f32 / stride as f32).ceil() as usize,
            ),
            Padding::Valid => (
                ((in_rows - filter_rows + 1) as f32 / stride as f32).ceil() as usize,
                ((in_cols - filter_cols + 1) as f32 / stride as f32).ceil() as usize,
            ),
        }
    }

    fn pad<T>(
        &self,
        data: ArrayView4<T>,
        shape: (usize, usize),
        item: T,
    ) -> Result<Option<Array4<T>>>
    where
        T: Copy + ::num_traits::Zero + ::std::fmt::Debug,
    {
        let img = BatchImageWrapper(data);
        let stride = self.strides[1];
        let (filter_rows, filter_cols) = shape;

        if self.padding == Padding::Same {
            // https://www.tensorflow.org/api_guides/python/nn#Convolution
            let v_padding = ::std::cmp::max(
                0,
                filter_rows -
                    if img.height() % stride == 0 {
                        stride
                    } else {
                        img.height() % stride
                    },
            );
            let h_padding = ::std::cmp::max(
                0,
                filter_cols -
                    if img.width() % stride == 0 {
                        stride
                    } else {
                        img.width() % stride
                    },
            );
            let left_padding = h_padding / 2;
            let right_padding = h_padding - left_padding;
            let top_padding = v_padding / 2;
            let bottom_padding = v_padding - top_padding;
            let left_padding = ::ndarray::Array4::<T>::from_elem(
                (img.count(), img.height(), left_padding, img.depth()),
                item,
            );
            let right_padding = ::ndarray::Array4::<T>::from_elem(
                (img.count(), img.height(), right_padding, img.depth()),
                item,
            );
            let tmp = ::ndarray::stack(
                ::ndarray::Axis(2),
                &[left_padding.view(), data.view(), right_padding.view()],
            )?;
            let top_padding = ::ndarray::Array4::<T>::from_elem(
                (img.count(), top_padding, tmp.shape()[2], img.depth()),
                item,
            );
            let bottom_padding = ::ndarray::Array4::<T>::from_elem(
                (img.count(), bottom_padding, tmp.shape()[2], img.depth()),
                item,
            );
            let a = ::ndarray::stack(
                ::ndarray::Axis(1),
                &[top_padding.view(), tmp.view(), bottom_padding.view()],
            )?;
            Ok(Some(a))
        } else {
            Ok(None)
        }
    }

    // data is expected in HWC
    fn mk_patches<T: Copy + ::num_traits::Zero + ::std::fmt::Debug>(
        &self,
        data: ArrayView<T, Ix3>,
        shape: (usize, usize),
    ) -> Result<Array2<T>> {
        if self.strides.len() != 4 || self.strides[0] != 1 && self.strides[3] != 1 ||
            self.strides[1] != self.strides[2]
        {
            Err(format!(
                "strides must be of the form [1, s, s, 1], found {:?}",
                self.strides
            ))?
        }
        let img = ImageWrapper(data);
        let stride = self.strides[1];
        let (filter_rows, filter_cols) = shape;

        let (out_height, out_width) =
            self.adjusted_dim(img.height(), img.width(), (filter_rows, filter_cols));

        let patches_size = (
            (out_height * out_width) as usize,
            filter_rows * filter_cols * img.depth(),
        );

        let mut patches = unsafe { ::ndarray::Array2::<T>::uninitialized(patches_size) };
        let data = data.into_shape((1, img.height(), img.width(), img.depth()))?;
        let padded = self.pad(data, (filter_rows, filter_cols), T::zero())?;
        let data = padded.as_ref().map(|a| a.view()).unwrap_or(data.view());
        for i_x in 0..out_width {
            for i_y in 0..out_height {
                let mut patch_row = patches.row_mut(i_y * out_width + i_x);
                for f_x in 0..filter_cols {
                    for f_y in 0..filter_rows {
                        for d in 0..img.depth() {
                            let loc = &mut patch_row[f_y * img.depth() * filter_cols +
                                                         f_x * img.depth() +
                                                         d];
                            *loc = data[(0, i_y * stride + f_y, i_x * stride + f_x, d)];
                        }
                    }
                }
            }
        }
        Ok(patches)
    }
}

#[derive(Debug)]
pub struct Conv2D(LocalPatch);

impl Conv2D {
    pub fn build(pb: &::tfpb::node_def::NodeDef) -> Result<Box<Op>> {
        Self::for_patch(LocalPatch::build(pb)?)
    }

    pub fn for_patch(patch: LocalPatch) -> Result<Box<Op>> {
        Ok(Box::new(Conv2D(patch)))
    }
}

impl Op for Conv2D {
    fn eval(&self, mut inputs: Vec<Input>) -> Result<Vec<Input>> {
        let (m_data, m_filter) = args_2!(inputs);
        let data = m_data.into_matrix().take_f32s().ok_or("Expected a f32 matrix")?;
        let filter = m_filter.as_f32s().ok_or("Expected a f32 matrix")?;

        let batches = data.shape()[0];
        let in_rows = data.shape()[1];
        let in_cols = data.shape()[2];
        let in_depth = data.shape()[3];
        let filter_rows = filter.shape()[0];
        let filter_cols = filter.shape()[1];
        let out_depth = filter.shape()[3];

        let (out_height, out_width) = self.0.adjusted_dim(
            in_rows,
            in_cols,
            (filter_rows, filter_cols),
        );

        let data = data.into_shape((batches, in_rows, in_cols, in_depth))?;
        let filter = ArrayView2::from_shape(
            (filter_rows * filter_cols * in_depth, out_depth),
            filter.as_slice().unwrap(),
        )?;

        let transformed: Vec<Array4<f32>> = data.outer_iter()
            .map(|image| -> Result<Array4<f32>> {
                let patches = self.0.mk_patches(image, (filter_rows, filter_cols))?;
                let transformed = patches.dot(&filter);
                Ok(transformed.into_shape(
                    (1, out_height, out_width, out_depth),
                )?)
            })
            .collect::<Result<Vec<Array4<f32>>>>()?;
        let views: Vec<ArrayView4<f32>> = transformed.iter().map(|m| m.view()).collect();
        Ok(vec![Matrix::from(::ndarray::stack(Axis(0), &*views)?.into_dyn()).into()])
    }
}

fn into_4d<T>(data: ArrayD<T>) -> Result<Array4<T>> {
    if data.shape().len() != 4 {
        Err(format!("Expeted 4D shape, found: {:?}", data.shape()))?
    }
    let shape = (
        data.shape()[0],
        data.shape()[1],
        data.shape()[2],
        data.shape()[3],
    );
    Ok(data.into_shape(shape)?)
}

#[derive(Debug)]
pub struct MaxPool(LocalPatch, (usize, usize));

impl MaxPool {
    pub fn build(pb: &::tfpb::node_def::NodeDef) -> Result<Box<Op>> {
        let ksize = pb.get_attr().get("ksize").unwrap().get_list().get_i();
        Ok(Box::new(MaxPool(
            LocalPatch::build(pb)?,
            (ksize[1] as usize, ksize[2] as usize),
        )))
    }
}

impl Op for MaxPool {
    fn eval(&self, mut inputs: Vec<Input>) -> Result<Vec<Input>> {
        let m_input = args_1!(inputs);
        let data = m_input.into_matrix().take_f32s().ok_or("Expected a f32 matrix")?;
        let data = into_4d(data)?;
        let images = BatchImageWrapper(data.view());

        let (out_h, out_w) = self.0.adjusted_dim(images.h(), images.w(), self.1);

        let h_stride = self.0.strides[1];
        let w_stride = self.0.strides[2];
        let padded = self.0.pad(data.view(), self.1, ::std::f32::NEG_INFINITY)?;
        let data = padded.as_ref().map(|a| a.view()).unwrap_or(data.view());
        let out_shape = (images.count(), out_h, out_w, images.d());

        let transformed = Array4::from_shape_fn(out_shape, |(b, h, w, d)| {
            let mut v = ::std::f32::NEG_INFINITY;
            for y in (h * h_stride)..(h * h_stride) + (self.1).0 {
                for x in (w * w_stride)..(w * w_stride) + (self.1).1 {
                    let v2 = data[(b, y, x, d)];
                    if v2 > v {
                        v = v2;
                    }
                }
            }
            v
        });

        Ok(vec![Matrix::from(transformed.into_dyn()).into()])
    }
}

#[derive(Debug)]
pub struct AvgPool(LocalPatch, (usize, usize));

impl AvgPool {
    pub fn build(pb: &::tfpb::node_def::NodeDef) -> Result<Box<Op>> {
        let ksize = pb.get_attr().get("ksize").unwrap().get_list().get_i();
        Ok(Box::new(AvgPool(
            LocalPatch::build(pb)?,
            (ksize[1] as usize, ksize[2] as usize),
        )))
    }
}

impl Op for AvgPool {
    fn eval(&self, mut inputs: Vec<Input>) -> Result<Vec<Input>> {
        let m_input = args_1!(inputs);
        let data = m_input.into_matrix().take_f32s().ok_or("Expected a f32 matrix")?;
        let data = into_4d(data)?;
        let images = BatchImageWrapper(data.view());

        let (out_h, out_w) = self.0.adjusted_dim(images.h(), images.w(), self.1);

        let h_stride = self.0.strides[1];
        let w_stride = self.0.strides[2];
        let padded = self.0.pad(data.view(), self.1, ::std::f32::NAN)?;
        let data = padded.as_ref().map(|a| a.view()).unwrap_or(data.view());
        let out_shape = (images.count(), out_h, out_w, images.d());

        let transformed = Array4::from_shape_fn(out_shape, |(b, h, w, d)| {
            let mut count = 0;
            let mut sum = 0.0;
            for y in (h * h_stride)..(h * h_stride) + (self.1).0 {
                for x in (w * w_stride)..(w * w_stride) + (self.1).1 {
                    let v = data[(b, y, x, d)];
                    if !v.is_nan() {
                        count += 1;
                        sum += v;
                    }
                }
            }
            sum / count as f32
        });

        Ok(vec![Matrix::from(transformed.into_dyn()).into()])
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use Matrix;
    use super::*;

    fn mk(sizes: &[usize]) -> Matrix {
        ::ndarray::Array::range(1f32, sizes.iter().product::<usize>() as f32 + 1.0, 1.0)
            .into_shape(sizes)
            .unwrap()
            .into()
    }

    fn verify(input: &[usize], filter: &[usize], stride: usize, padding: Padding, expect: &[f32]) {
        let strides = vec![1, stride, stride, 1];
        let result = Conv2D(LocalPatch {
            padding: padding,
            strides: strides,
            _data_format: DataFormat::NHWC,
        }).eval(vec![mk(input).into(), mk(filter).into()])
            .unwrap()
            .remove(0);
        assert_eq!(expect.len(), result.shape().iter().product::<usize>());
        let found = result.into_matrix()
            .take_f32s()
            .unwrap()
            .into_shape((expect.len()))
            .unwrap();
        assert_eq!(expect, found.as_slice().unwrap());
    }

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D1x1Filter() {
        verify(&[1,2,3,3], &[1, 1, 3, 3], 1, Padding::Valid, &[
        30.0, 36.0, 42.0, 66.0, 81.0, 96.0, 102.0, 126.0, 150.0, 138.0, 171.0,
        204.0, 174.0, 216.0, 258.0, 210.0, 261.0, 312.0 ]);
    }

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D1x2Filter() {
        verify(&[1, 2, 3, 3], &[1, 2, 3, 3] , 1, Padding::Valid, &[
        231.0, 252.0, 273.0, 384.0, 423.0, 462.0, 690.0, 765.0, 840.0, 843.0,
        936.0, 1029.0
    ])}

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D2x1Filter() {
        verify(&[1, 2, 3, 3], &[2, 1, 3, 3] , 1, Padding::Valid,
          &[465.0, 504.0, 543.0, 618.0, 675.0, 732.0, 771.0, 846.0, 921.0]);
    }

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D2x2Filter() {
        verify(&[1, 2, 3, 3], &[2, 2, 3, 3] , 1, Padding::Valid,
               &[ 2271.0, 2367.0, 2463.0, 2901.0, 3033.0, 3165.0 ])
    }

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D2x2FilterStride2() {
        verify(&[1, 2, 3, 3], &[2, 2, 3, 3] , 2, Padding::Valid,
               &[2271.0, 2367.0, 2463.0])
    }

    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn testConv2D2x2FilterStride2Same() {
        verify(&[1, 2, 3, 3], &[2, 2, 3, 3] , 2, Padding::Same,
               &[2271.0, 2367.0, 2463.0, 1230.0, 1305.0, 1380.0]);
    }

    #[test]
    fn test_conv_1() {
        let conv = Conv2D(LocalPatch {
            padding: Padding::Same,
            strides: vec![1, 1, 1, 1],
            _data_format: DataFormat::NHWC,
        });
        // NHWC
        let data: Matrix = Matrix::f32s(&[1, 1, 1, 1], &[1f32]).unwrap();
        // HWIO
        let filter = Matrix::f32s(&[3, 1, 1, 1], &[0.0, 1.0, 0.0]).unwrap();
        let exp: Matrix = Matrix::f32s(&[1, 1, 1, 1], &[1.0]).unwrap();

        let result = conv.eval(vec![data.into(), filter.into()]).unwrap().remove(0);
        assert_eq!(exp, result.into_matrix());
    }


    #[test]
    fn test_conv_2() {
        let conv = Conv2D(LocalPatch {
            padding: Padding::Same,
            strides: vec![1, 1, 1, 1],
            _data_format: DataFormat::NHWC,
        });
        let data = Matrix::f32s(&[1, 2, 2, 1], &[142.3088, 48.891083, 208.3187, -11.274994])
            .unwrap();
        let filter: Matrix = Matrix::f32s(
            &[2, 2, 1, 1],
            &[160.72833, 107.84076, 247.50552, -38.738464],
        ).unwrap();
        let exp: Matrix = Matrix::f32s(&[1, 2, 2, 1], &[80142.31, 5067.5586, 32266.81, -1812.2109])
            .unwrap();

        assert!(exp.close_enough(
            &conv.eval(vec![data.into(), filter.into()]).unwrap()[0],
        ))
    }

    #[test]
    fn test_maxpool_1() {
        let pool = MaxPool(
            LocalPatch {
                padding: Padding::Same,
                strides: vec![1, 1, 1, 1],
                _data_format: DataFormat::NHWC,
            },
            (2, 1),
        );
        let data = Matrix::f32s(&[1, 1, 1, 1], &[-1.0]).unwrap();
        let exp: Matrix = Matrix::f32s(&[1, 1, 1, 1], &[-1.0]).unwrap();
        let found = pool.eval(vec![data.into()]).unwrap();

        assert!(
            exp.close_enough(&found[0]),
            "expected: {:?} found: {:?}",
            exp,
            found[0]
        )
    }

    #[test]
    fn test_maxpool_2() {
        let pool = MaxPool(
            LocalPatch {
                padding: Padding::Same,
                strides: vec![1, 3, 3, 1],
                _data_format: DataFormat::NHWC,
            },
            (3, 3),
        );
        let data = Matrix::f32s(&[1, 2, 4, 1], &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).unwrap();
        let exp: Matrix = Matrix::f32s(&[1, 1, 2, 1], &[1.0, 0.0]).unwrap();
        let found = pool.eval(vec![data.into()]).unwrap();

        assert!(
            exp.close_enough(&found[0]),
            "expected: {:?} found: {:?}",
            exp,
            found[0]
        )
    }
}

#[cfg(all(test, feature = "tensorflow"))]
mod proptests {
    #![allow(non_snake_case)]
    use proptest::prelude::*;
    use ndarray::prelude::*;
    use protobuf::core::Message;
    use tfpb;
    use tfpb::types::DataType::DT_FLOAT;

    use Matrix;

    fn placeholder(name: &str) -> tfpb::node_def::NodeDef {
        tfpb::node().name(name).op("Placeholder").attr(
            "dtype",
            DT_FLOAT,
        )
    }

    fn maxpool_pb(
        v_stride: usize,
        h_stride: usize,
        kw: usize,
        kh: usize,
        valid: bool,
    ) -> ::Result<Vec<u8>> {
        let pool = tfpb::node()
            .name("pool")
            .op("MaxPool")
            .input("data")
            .attr("T", DT_FLOAT)
            .attr("strides", vec![1, v_stride as i64, h_stride as i64, 1])
            .attr("ksize", vec![1, kw as i64, kh as i64, 1])
            .attr("padding", if valid { "VALID" } else { "SAME" });

        let graph = tfpb::graph().node(placeholder("data")).node(pool);

        Ok(graph.write_to_bytes()?)
    }

    fn convolution_pb(v_stride: usize, h_stride: usize, valid: bool) -> ::Result<Vec<u8>> {

        let conv = tfpb::node()
            .name("conv")
            .op("Conv2D")
            .input("data")
            .input("kernel")
            .attr("strides", vec![1, v_stride as i64, h_stride as i64, 1])
            .attr("padding", if valid { "VALID" } else { "SAME" })
            .attr("T", DT_FLOAT);

        let graph = tfpb::graph()
            .node(placeholder("data"))
            .node(placeholder("kernel"))
            .node(conv);

        Ok(graph.write_to_bytes()?)
    }

    fn img_and_ker(
        ih: usize,
        iw: usize,
        ic: usize,
        kh: usize,
        kw: usize,
        kc: usize,
    ) -> BoxedStrategy<(Matrix, Matrix)> {
        (1..ih, 1..iw, 1..ic, 1..kh, 1..kw, 1..kc)
            .prop_flat_map(|(ih, iw, ic, kh, kw, kc)| {
                let i_size = iw * ih * ic;
                let k_size = kw * kh * kc * ic;
                (
                    Just(ih),
                    Just(iw),
                    Just(ic),
                    Just(kh),
                    Just(kw),
                    Just(kc),
                    ::proptest::collection::vec(-255f32..255f32, i_size..i_size + 1),
                    ::proptest::collection::vec(-255f32..255f32, k_size..k_size + 1),
                )
            })
            .prop_map(|(ih, iw, ic, kh, kw, kc, img, ker)| {
                (
                    Matrix::F32(
                        Array::from_vec(img)
                            .into_shape((1, ih, iw, ic))
                            .unwrap()
                            .into_dyn(),
                    ),
                    Matrix::F32(
                        Array::from_vec(ker)
                            .into_shape((kh, kw, ic, kc))
                            .unwrap()
                            .into_dyn(),
                    ),
                )
            })
            .boxed()
    }

    fn img_and_pool(
        ih: usize,
        iw: usize,
        ic: usize,
        kh: usize,
        kw: usize,
    ) -> BoxedStrategy<(Matrix, usize, usize)> {
        (1..ih, 1..iw, 1..ic, 1..kh, 1..kw)
            .prop_flat_map(|(ih, iw, ic, kh, kw)| {
                let i_size = iw * ih * ic;
                (
                    Just(ih),
                    Just(iw),
                    Just(ic),
                    Just(kh),
                    Just(kw),
                    ::proptest::collection::vec(-255f32..255f32, i_size..i_size + 1),
                )
            })
            .prop_map(|(ih, iw, ic, kh, kw, img)| {
                (
                    Matrix::F32(
                        Array::from_vec(img)
                            .into_shape((1, ih, iw, ic))
                            .unwrap()
                            .into_dyn(),
                    ),
                    kw,
                    kh,
                )
            })
            .boxed()
    }

    proptest! {
        #[test]
        fn conv((ref i, ref k) in img_and_ker(32, 32, 5, 16, 16, 8),
                           valid in ::proptest::bool::ANY,
                           stride in 1usize..4) {
            prop_assume!(stride <= k.shape()[0]);
            prop_assume!(stride <= k.shape()[1]);
            if valid {
                prop_assume!(i.shape()[1] >= k.shape()[0]);
                prop_assume!(i.shape()[2] >= k.shape()[1]);
            }
            let model = convolution_pb(stride, stride, valid).unwrap();
            let mut tf = ::tf::for_slice(&model)?;
            let tfd = ::Model::for_reader(&*model)?;
            let mut tfd = tfd.state();
            let expected = tf.run(vec!(("data", i.clone()), ("kernel", k.clone())), "conv")?;
            tfd.set_value("data", i.clone())?;
            tfd.set_value("kernel", k.clone())?;
            let found = tfd.take("conv")?;
            prop_assert!(expected[0].close_enough(&found[0]))
        }
    }

    proptest! {
        #[test]
        fn maxpool((ref i, kh, kw) in img_and_pool(32, 32, 5, 16, 16),
                           valid in ::proptest::bool::ANY,
                           stride in 1usize..4) {
            prop_assume!(stride <= kh);
            prop_assume!(stride <= kw);
            if valid {
                prop_assume!(i.shape()[1] >= kh);
                prop_assume!(i.shape()[2] >= kw);
            }
            let model = maxpool_pb(stride, stride, kh, kw, valid).unwrap();
            let mut tf = ::tf::for_slice(&model)?;
            let tfd = ::Model::for_reader(&*model)?;
            let mut tfd = tfd.state();
            let expected = tf.run(vec!(("data", i.clone())), "pool")?;
            tfd.set_value("data", i.clone())?;
            let found = tfd.take("pool")?;
            prop_assert!(expected[0].close_enough(&found[0]), "expected: {:?} found: {:?}", expected, found)
        }
    }
}