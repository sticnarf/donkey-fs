use super::DkResult;
use failure::Fail;

pub trait IntoByteResult {
    fn into(self) -> DkResult<u8>;
}

impl IntoByteResult for u8 {
    fn into(self) -> DkResult<u8> {
        Ok(self)
    }
}

impl<F> IntoByteResult for Result<u8, F>
where
    F: Fail,
{
    fn into(self) -> DkResult<u8> {
        Ok(self?)
    }
}

pub trait Block {
    fn from_bytes<I, U>(bytes: I) -> DkResult<Self>
    where
        Self: Sized,
        I: Iterator<Item = U>,
        U: IntoByteResult;

    fn as_bytes(&self) -> &[u8];
}
