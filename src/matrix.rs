use ndarray::prelude::*;

#[derive(Clone, PartialEq)]
pub enum Matrix {
    F32(ArrayD<f32>),
    I32(ArrayD<i32>),
    I8(ArrayD<i8>),
    U8(ArrayD<u8>),
    String(ArrayD<i8>),
}

impl Matrix {
    pub fn from_pb(t: &::tfpb::tensor::TensorProto) -> ::Result<Matrix> {
        use tfpb::types::DataType::*;
        use ndarray::*;
        let dtype = t.get_dtype();
        let shape = t.get_tensor_shape();
        let dims = shape.get_dim();
        let dims = if dims.len() == 0 {
            vec![1]
        } else {
            dims.iter().map(|d| d.size as usize).collect()
        };
        let content = t.get_tensor_content();
        if content.len() == 0 {
            match dtype {
                DT_INT32 => Ok(
                    Array1::from_iter(t.get_int_val().iter().cloned())
                        .into_dyn()
                        .into(),
                ),
                DT_FLOAT => Ok(
                    Array1::from_iter(t.get_float_val().iter().cloned())
                        .into_dyn()
                        .into(),
                ),
                DT_STRING => {
                    if t.get_string_val().len() != 1 {
                        Err(format!("Multiple string tensor not supported"))?
                    }
                    Ok(Matrix::U8(
                        Array1::from_iter(t.get_string_val()[0].iter().cloned())
                            .into_dyn(),
                    ))
                }
                _ => Err(format!("Missing simple tensor parser: type:{:?}", dtype))?,
            }
        } else {
            match dtype {
                DT_FLOAT => Ok(Self::from_content::<f32>(dims, content)?.into()),
                DT_INT32 => Ok(Self::from_content::<i32>(dims, content)?.into()),
                _ => {
                    Err(format!(
                        "Missing tensor parser: dims:{:?} type:{:?}, content.len:{}",
                        dims,
                        dtype,
                        content.len()
                    ))?
                }
            }
        }
    }

    pub fn from_content<T: Copy>(dims: Vec<usize>, content: &[u8]) -> ::Result<ArrayD<T>> {
        let value: &[T] = unsafe {
            ::std::slice::from_raw_parts(
                content.as_ptr() as _,
                content.len() / ::std::mem::size_of::<T>(),
            )
        };
        Ok(
            Array1::from_iter(value.iter().cloned())
                .into_shape(dims)?
                .into_dyn(),
        )
    }

    pub fn shape(&self) -> &[usize] {
        match self {
            &Matrix::I32(ref it) => it.shape(),
            &Matrix::F32(ref it) => it.shape(),
            &Matrix::U8(ref it) => it.shape(),
            _ => unimplemented!(),
        }
    }

    pub fn datatype(&self) -> ::tfpb::types::DataType {
        use tfpb::types::DataType;
        match self {
            &Matrix::I32(_) => DataType::DT_INT32,
            &Matrix::F32(_) => DataType::DT_FLOAT,
            &Matrix::U8(_) => DataType::DT_UINT8,
            _ => unimplemented!(),
        }
    }

    pub fn partial_dump(&self, single_line:bool) -> ::Result<String> {
        use std::io::Write;
        use std::io::BufRead;
        let mut w = Vec::new();
        match self {
            &Matrix::I32(ref a) => writeln!(&mut w, "I32 {:?}", a),
            &Matrix::F32(ref a) => writeln!(&mut w, "F32 {:?}", a),
            &Matrix::U8(ref a) => writeln!(&mut w, "U8 {:?}", a),
            _ => unimplemented!(),
        }?;
        let mut lines: Vec<String> = ::std::io::BufReader::new(&*w)
            .lines()
            .collect::<::std::io::Result<Vec<_>>>()?;
        if lines.len() > 5 {
            lines[2] = (if single_line { "..." } else { " : : :" }).into();
            while lines.len() > 5 {
                lines.remove(3);
            }
        }
        Ok(lines.iter().map(|s| s.trim().to_string() + if single_line {""} else {"\n"}).collect())
    }
}

impl ::std::fmt::Debug for Matrix {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "{}", self.partial_dump(true).map_err(|_| ::std::fmt::Error::default())?)
    }
}

pub trait CastFrom<T>
where
    Self: Sized,
{
    fn cast_from(value: T) -> Option<Self>;
}

pub trait CastInto<U> {
    fn cast_into(self) -> Option<U>;
}

impl<T, U> CastInto<U> for T
where
    U: CastFrom<T>,
{
    fn cast_into(self) -> Option<U> {
        U::cast_from(self)
    }
}

macro_rules! matrix {
    ($t:ident,$v:ident,$as:ident,$take:ident) => {
        impl From<ArrayD<$t>> for Matrix {
            fn from(it: ArrayD<$t>) -> Matrix {
                Matrix::$v(it)
            }
        }

        impl Matrix {
            pub fn $as(&self) -> Option<&ArrayD<$t>> {
                if let &Matrix::$v(ref it) = self {
                    Some(it)
                } else {
                    None
                }
            }

            pub fn $take(self) -> Option<ArrayD<$t>> {
                if let Matrix::$v(it) = self {
                    Some(it)
                } else {
                    None
                }
            }
        }

        impl CastFrom<Matrix> for ArrayD<$t> {
            fn cast_from(mat: Matrix) -> Option<ArrayD<$t>> {
                if let Matrix::$v(it) = mat {
                    Some(it)
                } else {
                    None
                }
            }
        }
    }
}

matrix!(f32, F32, as_f32s, take_f32s);
matrix!(i32, I32, as_i32s, take_i32s);
matrix!(u8, U8, as_u8s, take_u8s);
