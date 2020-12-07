//! `Tensor`, tract main data object of interest.
use crate::datum::{Blob, Datum, DatumType};
use crate::dim::TDim;
use crate::f16::f16;
use crate::TVec;
use ndarray::prelude::*;
#[cfg(feature = "serialize")]
use serde::ser::{Serialize, Serializer};
use std::alloc;
use std::borrow::Cow;
use std::fmt;
use std::hash::Hash;
use std::mem::{align_of, size_of};
use std::ops::Range;
use std::sync::Arc;

pub mod litteral;
pub mod view;

/// Tensor is a concrete tensor in tract.
pub struct Tensor {
    dt: DatumType,
    shape: TVec<usize>,
    strides: TVec<isize>,
    layout: alloc::Layout,
    data: *mut u8,
}

unsafe impl Send for Tensor {}
unsafe impl Sync for Tensor {}

impl Hash for Tensor {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use DatumType::*;
        self.dt.hash(state);
        self.shape.hash(state);
        self.layout.align().hash(state);
        unsafe {
            match self.dt {
                Bool => self.as_slice_unchecked::<bool>().hash(state),
                I8 => self.as_slice_unchecked::<i8>().hash(state),
                I16 => self.as_slice_unchecked::<i16>().hash(state),
                I32 => self.as_slice_unchecked::<i32>().hash(state),
                I64 => self.as_slice_unchecked::<i64>().hash(state),
                U8 => self.as_slice_unchecked::<u8>().hash(state),
                U16 => self.as_slice_unchecked::<u16>().hash(state),
                U32 => self.as_slice_unchecked::<u32>().hash(state),
                U64 => self.as_slice_unchecked::<u64>().hash(state),
                F16 => self.as_slice_unchecked::<i16>().hash(state),
                F32 => self.as_slice_unchecked::<i32>().hash(state),
                F64 => self.as_slice_unchecked::<i64>().hash(state),
                TDim => self.as_slice_unchecked::<crate::dim::TDim>().hash(state),
                String => self.as_slice_unchecked::<std::string::String>().hash(state),
                Blob => self.as_slice_unchecked::<crate::datum::Blob>().hash(state),
            }
        }
    }
}

impl Clone for Tensor {
    fn clone(&self) -> Tensor {
        self.deep_clone()
    }
}

impl Default for Tensor {
    fn default() -> Tensor {
        litteral::tensor0(0f32)
    }
}

impl Drop for Tensor {
    fn drop(&mut self) {
        if self.dt == DatumType::Blob {
            unsafe {
                self.as_slice_mut::<Blob>()
                    .unwrap()
                    .iter_mut()
                    .for_each(|s| std::ptr::drop_in_place(s as *mut Blob));
            }
        }
        if self.dt == DatumType::String {
            unsafe {
                self.as_slice_mut::<String>()
                    .unwrap()
                    .iter_mut()
                    .for_each(|s| std::ptr::drop_in_place(s as *mut String));
            }
        }
        if self.dt == DatumType::TDim {
            unsafe {
                self.as_slice_mut::<TDim>()
                    .unwrap()
                    .iter_mut()
                    .for_each(|s| std::ptr::drop_in_place(s as *mut TDim));
            }
        }
        if !self.data.is_null() && self.layout.size() > 0 {
            unsafe { alloc::dealloc(self.data, self.layout) }
        }
    }
}

impl Tensor {
    /// Create an uninitialized tensor (dt as type paramater).
    pub unsafe fn uninitialized<T: Datum>(shape: &[usize]) -> anyhow::Result<Tensor> {
        Self::uninitialized_dt(T::datum_type(), shape)
    }

    /// Create an uninitialized tensor (dt as regular parameter).
    pub unsafe fn uninitialized_dt(dt: DatumType, shape: &[usize]) -> anyhow::Result<Tensor> {
        Self::uninitialized_aligned_dt(dt, shape, dt.alignment())
    }

    /// Create an uninitialized tensor with a given alignment (in bytes).
    pub unsafe fn uninitialized_aligned<T: Datum>(
        shape: &[usize],
        alignment: usize,
    ) -> anyhow::Result<Tensor> {
        Self::uninitialized_aligned_dt(T::datum_type(), shape, alignment)
    }

    /// Create an uninitialized tensor with a given alignment (in bytes).
    pub unsafe fn uninitialized_aligned_dt(
        dt: DatumType,
        shape: &[usize],
        alignment: usize,
    ) -> anyhow::Result<Tensor> {
        if dt == String::datum_type() {
            return Ok(ndarray::ArrayD::<String>::default(shape).into());
        } else if dt == TDim::datum_type() {
            return Ok(ndarray::ArrayD::<TDim>::default(shape).into());
        }
        let bytes = shape.iter().cloned().product::<usize>() * dt.size_of();
        let layout = alloc::Layout::from_size_align(bytes, alignment)?;
        let data = if bytes == 0 {
            std::ptr::null()
        } else {
            let ptr = alloc::alloc(layout);
            assert!(!ptr.is_null());
            ptr
        } as *mut u8;
        let mut tensor = Tensor { strides: tvec!(), layout, dt, shape: shape.into(), data };
        tensor.update_strides();
        Ok(tensor)
    }

    pub fn stack_tensors(
        axis: usize,
        tensors: &[impl std::borrow::Borrow<Tensor>],
    ) -> anyhow::Result<Tensor> {
        use crate::datum::ArrayDatum;
        let dt = tensors[0].borrow().datum_type();
        if tensors.iter().any(|t| t.borrow().datum_type() != dt) {
            anyhow::bail!("Inconsistent datum type in stack.")
        }
        // map all copy types to the i* of the same size
        let mut tensor = unsafe {
            match dt {
                DatumType::F16 => i16::stack_tensors(axis, &tensors),
                DatumType::F32 => i32::stack_tensors(axis, &tensors),
                DatumType::F64 => i64::stack_tensors(axis, &tensors),
                DatumType::Bool => i8::stack_tensors(axis, &tensors),
                DatumType::U8 => i8::stack_tensors(axis, &tensors),
                DatumType::U16 => i16::stack_tensors(axis, &tensors),
                DatumType::U32 => i32::stack_tensors(axis, &tensors),
                DatumType::U64 => i64::stack_tensors(axis, &tensors),
                DatumType::I8 => i8::stack_tensors(axis, &tensors),
                DatumType::I16 => i16::stack_tensors(axis, &tensors),
                DatumType::I32 => i32::stack_tensors(axis, &tensors),
                DatumType::I64 => i64::stack_tensors(axis, &tensors),
                DatumType::TDim => TDim::stack_tensors(axis, &tensors),
                DatumType::Blob => Blob::stack_tensors(axis, &tensors),
                DatumType::String => String::stack_tensors(axis, &tensors),
            }
        }?;
        tensor.dt = dt;
        Ok(tensor)
    }

    pub unsafe fn clear<T: Datum + num_traits::Zero>(&mut self) {
        self.as_slice_mut_unchecked::<T>().iter_mut().for_each(|item| *item = T::zero());
    }

    pub fn zero<T: Datum + num_traits::Zero>(shape: &[usize]) -> anyhow::Result<Tensor> {
        unsafe {
            let mut t = Tensor::uninitialized::<T>(shape)?;
            t.clear::<T>();
            Ok(t)
        }
    }

    pub fn zero_dt(dt: DatumType, shape: &[usize]) -> anyhow::Result<Tensor> {
        dispatch_numbers!(Self::zero(dt)(shape))
    }

    pub fn zero_aligned_dt(
        dt: DatumType,
        shape: &[usize],
        alignment: usize,
    ) -> anyhow::Result<Tensor> {
        dispatch_numbers!(Self::zero_aligned(dt)(shape, alignment))
    }

    pub fn zero_aligned<T: Datum + num_traits::Zero>(
        shape: &[usize],
        alignment: usize,
    ) -> anyhow::Result<Tensor> {
        unsafe {
            let mut tensor = Self::uninitialized_aligned::<T>(shape, alignment)?;
            tensor.clear::<T>();
            Ok(tensor)
        }
    }

    /// Create an tensor from raw data.
    ///
    /// It copies the data, aligning it to the size of T.
    pub unsafe fn from_raw<T: Datum>(shape: &[usize], content: &[u8]) -> anyhow::Result<Tensor> {
        Tensor::from_raw_dt(T::datum_type(), shape, content)
    }

    pub unsafe fn from_raw_aligned<T: Datum>(
        shape: &[usize],
        content: &[u8],
        align: usize,
    ) -> anyhow::Result<Tensor> {
        Tensor::from_raw_dt_align(T::datum_type(), shape, content, align)
    }

    pub unsafe fn from_raw_dt(
        dt: DatumType,
        shape: &[usize],
        content: &[u8],
    ) -> anyhow::Result<Tensor> {
        Self::from_raw_dt_align(dt, shape, content, dt.alignment())
    }

    pub unsafe fn from_raw_dt_align(
        dt: DatumType,
        shape: &[usize],
        content: &[u8],
        align: usize,
    ) -> anyhow::Result<Tensor> {
        let mut tensor = Tensor::uninitialized_aligned_dt(dt, shape, align)?;
        tensor.as_bytes_mut().copy_from_slice(content);
        Ok(tensor)
    }

    pub unsafe fn from_slice_align<T: Datum>(
        content: &[T],
        align: usize,
    ) -> anyhow::Result<Tensor> {
        let bytes = std::slice::from_raw_parts(
            content.as_ptr() as *const u8,
            content.len() * T::datum_type().size_of(),
        );
        Self::from_raw_dt_align(T::datum_type(), &[content.len()], bytes, align)
    }

    /// Get the number of dimensions (or axes) of the tensor.
    pub fn rank(&self) -> usize {
        self.shape.len()
    }

    /// Get the shape of the tensor.
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// Get the number of valeus in the tensor.
    pub fn len(&self) -> usize {
        self.shape.iter().cloned().product::<usize>()
    }

    /// Get the shape of the tensor.
    pub fn strides(&self) -> &[isize] {
        &self.strides
    }

    fn update_strides(&mut self) {
        self.strides.clear();
        compute_natural_stride_to(&mut self.strides, &self.shape);
    }

    /// Force the tensor shape, no consistency check.
    pub unsafe fn set_shape_unchecked(&mut self, shape: &[usize]) {
        if shape != &*self.shape {
            self.shape.clear();
            self.shape.extend_from_slice(shape);
            self.update_strides();
        }
    }

    /// Force the tensor shape.
    pub fn set_shape(&mut self, shape: &[usize]) -> anyhow::Result<()> {
        if self.len() != shape.iter().product::<usize>() {
            anyhow::bail!("Invalid reshape {:?} to {:?}", self.shape, shape);
        }
        unsafe { self.set_shape_unchecked(shape) }
        Ok(())
    }

    pub fn permute_axes(self, axes: &[usize]) -> anyhow::Result<Tensor> {
        unsafe {
            #[inline]
            unsafe fn permute<T: Datum>(axes: &[usize], input: Tensor) -> Tensor {
                input.into_array_unchecked::<T>().permuted_axes(axes).into_tensor()
            }
            let dt = self.datum_type();
            let mut t = dispatch_datum_by_size!(permute(self.datum_type())(axes, self));
            t.set_datum_type(dt);
            Ok(t)
        }
    }

    /// Reshape the tensor to `shape`.
    pub fn into_shape(mut self, shape: &[usize]) -> anyhow::Result<Tensor> {
        self.set_shape(shape)?;
        Ok(self)
    }

    pub fn insert_axis(&mut self, axis: usize) -> anyhow::Result<()> {
        self.shape.insert(axis, 1);
        self.strides.insert(axis, self.strides.get(axis).copied().unwrap_or(1));
        Ok(())
    }

    pub fn remove_axis(&mut self, axis: usize) -> anyhow::Result<()> {
        self.shape.remove(axis);
        self.strides.remove(axis);
        Ok(())
    }

    pub fn broadcast_into_rank(mut self, rank: usize) -> anyhow::Result<Tensor> {
        self.broadcast_to_rank(rank)?;
        self.update_strides();
        Ok(self)
    }

    pub fn broadcast_to_rank(&mut self, rank: usize) -> anyhow::Result<()> {
        if rank < self.rank() {
            anyhow::bail!("Can only broadcast to higher rank")
        }
        while self.shape.len() < rank {
            self.shape.insert(0, 1)
        }
        self.update_strides();
        Ok(())
    }

    pub fn broadcast_scalar_to_shape(&self, shape: &[usize]) -> anyhow::Result<Tensor> {
        if self.rank() > 0 {
            anyhow::bail!("broadcast_scalar_to_shape called on {:?}", self);
        }
        unsafe fn make<T: Datum>(src: &Tensor, dst: &mut Tensor) {
            let value: &T = src.to_scalar_unchecked::<T>();
            dst.as_slice_mut_unchecked::<T>().iter_mut().for_each(|item| *item = value.clone());
        }
        unsafe {
            let mut t = Tensor::uninitialized_dt(self.datum_type(), shape)?;
            dispatch_datum_by_size!(make(self.datum_type())(self, &mut t));
            Ok(t)
        }
    }

    fn clip_range_bounds(
        &self,
        axis: usize,
        range: impl std::ops::RangeBounds<usize>,
    ) -> Range<usize> {
        use std::ops::Bound;
        let start = match range.start_bound() {
            Bound::Included(ix) => *ix,
            Bound::Excluded(ix) => ix + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(ix) => *ix + 1,
            Bound::Excluded(ix) => *ix,
            Bound::Unbounded => self.shape()[axis],
        };
        start..end
    }

    pub fn assign_slice(
        &mut self,
        range: impl std::ops::RangeBounds<usize>,
        src: &Tensor,
        src_range: impl std::ops::RangeBounds<usize>,
        axis: usize,
    ) -> anyhow::Result<()> {
        let range = self.clip_range_bounds(axis, range);
        let src_range = src.clip_range_bounds(axis, src_range);
        anyhow::ensure!(
            src.datum_type() == self.datum_type(),
            "Attempt to assign into {:?} from {:?}, datum type mismatch",
            self.datum_type(),
            src.datum_type()
        );
        anyhow::ensure!(
            src_range.len() == range.len(),
            "Attempt to assign a range of {:?} from a range of {:?}",
            range,
            src_range,
        );
        anyhow::ensure!(
            self.rank() == src.rank()
                && itertools::izip!(0.., self.shape(), src.shape())
                    .all(|(ix, dst, src)| ix == axis || src == dst),
            "Attempt to assign a {}-axis range of {:?} from a range of {:?}",
            axis,
            self,
            src
        );
        anyhow::ensure!(
            src_range.end <= src.shape()[axis],
            "Assigning from invalid slice (axis {}, {:?}) of {:?}",
            axis,
            src_range,
            src
        );
        anyhow::ensure!(
            range.end <= self.shape()[axis],
            "Assigning to invalid slice (axis {}, {:?}) of {:?}",
            axis,
            range,
            self
        );
        self.assign_slice_from_resolved(range, src, src_range, axis);
        Ok(())
    }

    pub unsafe fn assign_slice_unchecked(
        &mut self,
        range: impl std::ops::RangeBounds<usize>,
        src: &Tensor,
        src_range: impl std::ops::RangeBounds<usize>,
        axis: usize,
    ) {
        let range = self.clip_range_bounds(axis, range);
        let src_range = src.clip_range_bounds(axis, src_range);
        self.assign_slice_from_resolved(range, src, src_range, axis);
    }

    fn assign_slice_from_resolved(
        &mut self,
        range: std::ops::Range<usize>,
        src: &Tensor,
        src_range: std::ops::Range<usize>,
        axis: usize,
    ) {
        use ndarray::Slice;
        unsafe fn assign_slice_t<T: Datum>(
            to: &mut Tensor,
            to_range: Range<usize>,
            from: &Tensor,
            from_range: Range<usize>,
            axis: usize,
        ) {
            to.to_array_view_mut_unchecked::<T>()
                .slice_axis_mut(Axis(axis), Slice::from(to_range))
                .assign(
                    &from
                        .to_array_view_unchecked::<T>()
                        .slice_axis(Axis(axis), Slice::from(from_range)),
                )
        }
        unsafe {
            if axis == 0 && self.datum_type().is_copy() {
                let stride = self.strides[0] as usize * self.datum_type().size_of();
                let dst_start = (stride * range.start) as isize;
                let src_start = (stride * src_range.start) as isize;
                let len = stride * range.len();
                if self.data != src.data {
                    std::ptr::copy_nonoverlapping(
                        src.data.offset(src_start),
                        self.data.offset(dst_start),
                        len,
                    );
                } else {
                    std::ptr::copy(src.data.offset(src_start), self.data.offset(dst_start), len);
                }
            } else {
                dispatch_datum!(assign_slice_t(self.datum_type())(
                    self, range, src, src_range, axis
                ));
            }
        }
    }

    /// Get the datum type of the tensor.
    pub fn datum_type(&self) -> DatumType {
        self.dt
    }

    /// Set the datum type of the tensor.
    pub unsafe fn set_datum_type(&mut self, dt: DatumType) {
        self.dt = dt
    }

    /// Dump the tensor in a human readable form.
    ///
    /// `force_full` will force the tensor to be dump in full even if it is big.
    pub fn dump(&self, force_full: bool) -> anyhow::Result<String> {
        use itertools::Itertools;
        unsafe fn dump_t<D: Datum>(tensor: &Tensor, n: usize) -> String {
            tensor.as_slice_unchecked::<D>()[0..n].iter().join(", ")
        }
        unsafe {
            let trunc = self.len() > 12 && !force_full;
            let data = dispatch_datum!(dump_t(self.datum_type())(
                self,
                if trunc { 12 } else { self.len() }
            ));
            Ok(format!(
                "{},{:?} {}{}",
                self.shape.iter().join("x"),
                self.dt,
                data,
                if trunc { "..." } else { "" }
            ))
        }
    }

    /// Compare two tensors, allowing for rounding errors.
    pub fn close_enough(&self, other: &Self, approx: bool) -> anyhow::Result<()> {
        if self.shape() != other.shape() {
            anyhow::bail!("Shape mismatch {:?} != {:?}", self.shape(), other.shape())
        }
        if approx {
            let atol = 5e-4;
            let rtol = 1e-4;
            let ma = self.cast_to::<f32>()?;
            let ma = ma.to_array_view::<f32>()?;
            let mb = other.cast_to::<f32>()?;
            let mb = mb.to_array_view::<f32>()?;
            ndarray::indices_of(&ma).into_iter().try_for_each(|indices| {
                let a = ma[&indices];
                let b = mb[&indices];
                if !((a.is_nan() && b.is_nan())
                    || (a.is_infinite() && b.is_infinite() && a.signum() == b.signum())
                    || (a - b).abs() <= atol + rtol * b.abs())
                {
                    anyhow::bail!("Mismatch at {:?} {} != {}", indices.slice(), a, b)
                }
                Ok(())
            })
        } else {
            if self.eq(other) {
                Ok(())
            } else {
                anyhow::bail!("Mismatch")
            }
        }
    }

    /// Transform the tensor into a `ndarray::Array`.
    pub fn into_array<D: Datum>(self) -> anyhow::Result<ArrayD<D>> {
        Ok(self.to_array_view::<D>()?.to_owned())
    }

    /// Transform the tensor into a `ndarray::Array`.
    pub unsafe fn into_array_unchecked<D: Datum>(self) -> ArrayD<D> {
        self.to_array_view_unchecked::<D>().to_owned()
    }

    fn check_for_access<D: Datum>(&self) -> anyhow::Result<()> {
        if self.datum_type() != D::datum_type() {
            anyhow::bail!(
                "Tensor datum type error: tensor is {:?}, accessed as {:?}",
                self.datum_type(),
                D::datum_type(),
            );
        }
        Ok(())
    }

    /// Transform the data as a `ndarray::Array`.
    pub fn to_array_view<'a, D: Datum>(&'a self) -> anyhow::Result<ArrayViewD<'a, D>> {
        self.check_for_access::<D>()?;
        unsafe { Ok(self.to_array_view_unchecked()) }
    }

    /// Transform the data as a mutable `ndarray::Array`.
    pub fn to_array_view_mut<'a, D: Datum>(&'a mut self) -> anyhow::Result<ArrayViewMutD<'a, D>> {
        self.check_for_access::<D>()?;
        unsafe { Ok(self.to_array_view_mut_unchecked()) }
    }

    /// Transform the data as a `ndarray::Array`.
    pub unsafe fn to_array_view_unchecked<'a, D: Datum>(&'a self) -> ArrayViewD<'a, D> {
        if self.len() != 0 {
            ArrayViewD::from_shape_ptr(&*self.shape, self.data as *const D)
        } else {
            ArrayViewD::from_shape(&*self.shape, &[]).unwrap()
        }
    }

    /// Transform the data as a mutable `ndarray::Array`.
    pub unsafe fn to_array_view_mut_unchecked<'a, D: Datum>(&'a mut self) -> ArrayViewMutD<'a, D> {
        if self.len() != 0 {
            ArrayViewMutD::from_shape_ptr(&*self.shape, self.data as *mut D)
        } else {
            ArrayViewMutD::from_shape(&*self.shape, &mut []).unwrap()
        }
    }

    /// Access the data as a pointer.
    pub fn as_ptr<D: Datum>(&self) -> anyhow::Result<*const D> {
        self.check_for_access::<D>()?;
        Ok(self.data as *const D)
    }

    /// Access the data as a pointer.
    pub unsafe fn as_ptr_unchecked<D: Datum>(&self) -> *const D {
        self.data as *const D
    }

    /// Access the data as a pointer.
    pub unsafe fn as_ptr_mut_unchecked<D: Datum>(&mut self) -> *mut D {
        self.data as *mut D
    }

    /// Access the data as a mutable pointer.
    pub fn as_ptr_mut<D: Datum>(&mut self) -> anyhow::Result<*mut D> {
        self.as_ptr::<D>().map(|p| p as *mut D)
    }

    /// Access the data as a slice.
    pub fn as_slice<D: Datum>(&self) -> anyhow::Result<&[D]> {
        unsafe { Ok(std::slice::from_raw_parts::<D>(self.as_ptr()?, self.len())) }
    }

    /// Access the data as a mutable slice.
    pub fn as_slice_mut<D: Datum>(&mut self) -> anyhow::Result<&mut [D]> {
        unsafe { Ok(std::slice::from_raw_parts_mut::<D>(self.as_ptr_mut()?, self.len())) }
    }

    /// Access the data as a slice.
    pub unsafe fn as_slice_unchecked<D: Datum>(&self) -> &[D] {
        std::slice::from_raw_parts::<D>(self.data as *const D, self.len())
    }

    /// Access the data as a mutable slice.
    pub unsafe fn as_slice_mut_unchecked<D: Datum>(&mut self) -> &mut [D] {
        std::slice::from_raw_parts_mut::<D>(self.data as *mut D, self.len())
    }

    /// Access the data as a scalar.
    pub fn to_scalar<'a, D: Datum>(&'a self) -> anyhow::Result<&D> {
        unsafe { Ok(&*(self.as_ptr::<D>()?)) }
    }

    /// Access the data as a scalar.
    pub unsafe fn to_scalar_unchecked<'a, D: Datum>(&'a self) -> &D {
        &*(self.data as *mut D)
    }

    pub unsafe fn as_bytes(&self) -> &[u8] {
        std::slice::from_raw_parts(self.data, self.layout.size())
    }

    pub unsafe fn as_bytes_mut(&mut self) -> &mut [u8] {
        std::slice::from_raw_parts_mut(self.data, self.layout.size())
    }

    fn is_uniform_t<T: Datum>(&self) -> anyhow::Result<bool> {
        let slice = self.as_slice::<T>()?;
        Ok(slice[1..].iter().all(|x| x == &slice[0]))
    }

    pub fn is_uniform(&self) -> anyhow::Result<bool> {
        if self.len() <= 1 {
            return Ok(true);
        }
        dispatch_datum!(Tensor::is_uniform_t(self.datum_type())(self))
    }

    unsafe fn natural_cast<
        Source: Datum + num_traits::AsPrimitive<Target>,
        Target: Datum + Copy,
    >(
        &self,
        other: &mut Tensor,
    ) {
        self.as_slice_unchecked::<Source>()
            .iter()
            .zip(other.as_slice_mut_unchecked::<Target>().iter_mut())
            .for_each(|(s, d)| *d = s.as_());
    }

    unsafe fn cast_number_to_bool<Source: Datum + num_traits::Zero>(&self, other: &mut Tensor) {
        self.as_slice_unchecked::<Source>()
            .iter()
            .zip(other.as_slice_mut_unchecked::<bool>().iter_mut())
            .for_each(|(s, d)| *d = !s.is_zero());
    }

    unsafe fn cast_from_string<Target: Datum + core::str::FromStr>(
        &self,
        other: &mut Tensor,
    ) -> anyhow::Result<()> {
        for (s, d) in self
            .as_slice_unchecked::<String>()
            .iter()
            .zip(other.as_slice_mut_unchecked::<Target>().iter_mut())
        {
            *d = s.parse().map_err(|_| {
                anyhow::format_err!("Could not parse {} as {:?}", s, Target::datum_type())
            })?
        }
        Ok(())
    }

    unsafe fn cast_to_string<Source: Datum>(&self, other: &mut Tensor) {
        for (s, d) in self
            .as_slice_unchecked::<Source>()
            .iter()
            .zip(other.as_slice_mut_unchecked::<String>().iter_mut())
        {
            *d = s.to_string()
        }
    }

    /// Optionnaly convert data to a tensor for a new DatumType.
    pub fn cast_to<D: Datum>(&self) -> anyhow::Result<Cow<Tensor>> {
        self.cast_to_dt(D::datum_type())
    }

    /// Optionnaly convert data to a tensor for a new DatumType.
    pub fn cast_to_dt(&self, dt: DatumType) -> anyhow::Result<Cow<Tensor>> {
        unsafe {
            if self.dt == dt {
                return Ok(Cow::Borrowed(self));
            }
            if self.dt == TDim::datum_type() && (dt.is_integer() || dt.is_float()) {
                let slice = self.as_slice_unchecked::<TDim>();
                let mut ints = Self::uninitialized::<i64>(&self.shape)?;
                let ints_slice = ints.as_slice_mut_unchecked::<i64>();
                for i in 0..self.len() {
                    ints_slice[i] = slice[i].to_i64()?;
                }
                return Ok(Cow::Owned(ints.cast_to_dt(dt)?.into_owned()));
            }
            if self.dt == bool::datum_type() && (dt.is_integer() || dt.is_float()) {
                let slice = self.as_slice_unchecked::<bool>();
                let mut ints = Self::uninitialized::<i8>(&self.shape)?;
                let ints_slice = ints.as_slice_mut_unchecked::<i8>();
                for i in 0..self.len() {
                    ints_slice[i] = slice[i] as usize as i8;
                }
                return Ok(Cow::Owned(ints.cast_to_dt(dt)?.into_owned()));
            }
            let mut result = Self::uninitialized_dt(dt, &self.shape)?;
            if self.dt == DatumType::String {
                dispatch_datum!(Self::cast_from_string(dt)(self, &mut result))?;
                return Ok(Cow::Owned(result));
            }
            if dt == DatumType::String {
                dispatch_datum!(Self::cast_to_string(self.dt)(self, &mut result));
                return Ok(Cow::Owned(result));
            }
            macro_rules! n {
                ($source:ty) => {
                    if <$source>::datum_type() == self.datum_type() {
                        match dt {
                            DatumType::I8 => self.natural_cast::<$source, i8>(&mut result),
                            DatumType::I16 => self.natural_cast::<$source, i16>(&mut result),
                            DatumType::I32 => self.natural_cast::<$source, i32>(&mut result),
                            DatumType::I64 => self.natural_cast::<$source, i64>(&mut result),
                            DatumType::U8 => self.natural_cast::<$source, u8>(&mut result),
                            DatumType::U16 => self.natural_cast::<$source, u16>(&mut result),
                            DatumType::U32 => self.natural_cast::<$source, u32>(&mut result),
                            DatumType::U64 => self.natural_cast::<$source, u64>(&mut result),
                            DatumType::F16 => self.natural_cast::<$source, f16>(&mut result),
                            DatumType::F32 => self.natural_cast::<$source, f32>(&mut result),
                            DatumType::F64 => self.natural_cast::<$source, f64>(&mut result),
                            DatumType::TDim => {
                                let ints = self.cast_to::<i32>()?;
                                let slice = ints.as_slice_unchecked::<i32>();
                                let result = result.as_slice_mut_unchecked::<TDim>();
                                for i in 0..self.len() {
                                    result[i] = slice[i].into();
                                }
                            }
                            DatumType::Bool => self.cast_number_to_bool::<$source>(&mut result),
                            _ => todo!(),
                        }
                        return Ok(Cow::Owned(result));
                    };
                };
            };
            n!(u8);
            n!(u16);
            n!(u32);
            n!(u64);
            n!(i8);
            n!(i16);
            n!(i32);
            n!(i64);
            n!(f16);
            n!(f32);
            n!(f64);
            anyhow::bail!("Unsupported cast from {:?} to {:?}", self.dt, dt)
        }
    }

    /// Access the data as a scalar, after a cast.
    pub fn cast_to_scalar<D: Datum + Copy>(&self) -> anyhow::Result<D> {
        let casted = self.cast_to::<D>()?;
        casted.to_scalar::<D>().map(|&x| x)
    }

    /// Access the nth element of the tensor, returned as a 0-rank Tensor
    pub fn nth(&self, nth: usize) -> anyhow::Result<Tensor> {
        if nth >= self.len() {
            anyhow::bail!(
                "nth called with {}th element on a tensor of len {} ({:?}",
                nth,
                self.len(),
                self
            );
        }
        unsafe fn nth_t<T: Datum>(me: &Tensor, nth: usize, output: &mut Tensor) {
            let value = me.as_slice_unchecked::<T>()[nth].clone();
            output.as_slice_mut_unchecked::<T>()[0] = value;
        }
        unsafe {
            let mut output = Tensor::uninitialized_dt(self.datum_type(), &[])?;
            dispatch_datum_by_size!(nth_t(self.datum_type())(self, nth, &mut output));
            Ok(output)
        }
    }

    /// Strict equality test on tensors.
    fn eq_dt(&self, other: &Tensor) -> anyhow::Result<bool> {
        unsafe fn eq_t<D: Datum>(me: &Tensor, other: &Tensor) -> bool {
            me.as_slice_unchecked::<D>() == other.as_slice_unchecked::<D>()
        }

        unsafe {
            Ok(self.datum_type() == other.datum_type()
                && self.shape() == other.shape()
                && dispatch_datum!(eq_t(self.dt)(self, other)))
        }
    }

    fn from_copy_datum<D: ::ndarray::Dimension, T: Datum>(it: Array<T, D>) -> Tensor {
        let shape = it.shape().into();
        let vec = if it.as_slice().is_some() {
            it.into_raw_vec().into_boxed_slice()
        } else {
            it.into_owned().into_iter().cloned().collect::<Box<[T]>>()
        };
        let layout =
            alloc::Layout::from_size_align(vec.len() * size_of::<T>(), align_of::<T>()).unwrap();
        let data = Box::into_raw(vec) as *mut u8;
        let mut t = Tensor { dt: T::datum_type(), shape, layout, data, strides: tvec!() };
        t.update_strides();
        t
    }

    pub fn deep_clone(&self) -> Tensor {
        if self.dt == DatumType::String {
            let data: Vec<String> = self.as_slice::<String>().unwrap().to_vec();
            let t = Tensor {
                data: data.as_ptr() as *mut u8,
                shape: self.shape.clone(),
                strides: self.strides.clone(),
                ..*self
            };
            std::mem::forget(data);
            t
        } else if self.dt == DatumType::TDim {
            let data: Vec<TDim> = self.as_slice::<TDim>().unwrap().to_vec();
            let t = Tensor {
                data: data.as_ptr() as *mut u8,
                shape: self.shape.clone(),
                strides: self.strides.clone(),
                ..*self
            };
            std::mem::forget(data);
            t
        } else {
            unsafe {
                let tensor = Tensor::uninitialized_dt(self.datum_type(), self.shape()).unwrap();
                self.data
                    .copy_to_nonoverlapping(tensor.data, self.len() * self.datum_type().size_of());
                tensor
            }
        }
    }

    pub fn slice(&self, axis: usize, start: usize, end: usize) -> anyhow::Result<Tensor> {
        if axis >= self.rank() {
            anyhow::bail!("Can not slice at axis {} tensor {:?}", axis, self);
        }
        fn slice_t<T: Datum>(
            t: &Tensor,
            axis: usize,
            start: usize,
            end: usize,
        ) -> anyhow::Result<Tensor> {
            Ok(t.to_array_view::<T>()?
                .slice_axis(ndarray::Axis(axis), (start..end).into())
                .into_owned()
                .into_tensor())
        }
        dispatch_datum!(slice_t(self.datum_type())(&self, axis, start, end))
    }

    pub fn view(&self) -> view::TensorView {
        unsafe { view::TensorView::at_prefix_unchecked(self, &[]) }
    }

    pub fn view_at_prefix(&self, prefix: &[usize]) -> anyhow::Result<view::TensorView> {
        view::TensorView::at_prefix(self, prefix)
    }

    pub fn view_mut(&mut self) -> view::TensorView {
        unsafe { view::TensorView::at_prefix_unchecked(self, &[]) }
    }

    pub fn view_at_prefix_mut(&mut self, prefix: &[usize]) -> anyhow::Result<view::TensorView> {
        view::TensorView::at_prefix(self, prefix)
    }
}

impl PartialEq for Tensor {
    fn eq(&self, other: &Tensor) -> bool {
        if self.dt != other.dt || self.shape != other.shape {
            return false;
        }
        self.eq_dt(other).unwrap_or(false)
    }
}

impl fmt::Debug for Tensor {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let content = self.dump(false).unwrap_or_else(|e| format!("Error : {:?}", e));
        write!(formatter, "{}", content)
    }
}

pub fn natural_strides(shape: &[usize]) -> TVec<isize> {
    let mut strides = tvec!();
    compute_natural_stride_to(&mut strides, shape);
    strides
}

fn compute_natural_stride_to(strides: &mut TVec<isize>, shape: &[usize]) {
    match shape.len() {
        0 => (),
        1 => strides.push(1),
        2 => strides.extend_from_slice(&[shape[1] as isize, 1]),
        3 => strides.extend_from_slice(&[(shape[1] * shape[2]) as isize, shape[2] as _, 1]),
        4 => strides.extend_from_slice(&[
            (shape[1] * shape[2] * shape[3]) as isize,
            (shape[2] * shape[3]) as _,
            shape[3] as _,
            1,
        ]),
        _ => {
            strides.push(1);
            for dim in shape.as_ref().iter().skip(1).rev() {
                let previous = strides.last().unwrap().clone();
                strides.push(previous * *dim as isize)
            }
            strides.reverse();
        }
    }
}

#[cfg(feature = "serialize")]
impl Serialize for Tensor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        macro_rules! serialize_inner {
            ($type:ident, $m:ident) => {{
                let data =
                    (stringify!($type), self.shape(), $m.iter().cloned().collect::<Vec<_>>());
                data.serialize(serializer)
            }};
        };

        use Tensor::*;
        match self {
            Bool(m) => serialize_inner!(bool, m),
            U8(m) => serialize_inner!(u8, m),
            U16(m) => serialize_inner!(u16, m),
            I8(m) => serialize_inner!(i8, m),
            I16(m) => serialize_inner!(i16, m),
            I32(m) => serialize_inner!(i32, m),
            I64(m) => serialize_inner!(i64, m),
            F16(m) => serialize_inner!(f16, m),
            F32(m) => serialize_inner!(f32, m),
            F64(m) => serialize_inner!(f64, m),
            TDim(m) => serialize_inner!(TDim, m),
            String(m) => serialize_inner!(str, m),
        }
    }
}

impl<D: ::ndarray::Dimension, T: Datum> From<Array<T, D>> for Tensor {
    fn from(it: Array<T, D>) -> Tensor {
        Tensor::from_copy_datum(it)
    }
}

/// Convenient conversion to Tensor.
pub trait IntoTensor: Sized {
    /// Convert Self to a Tensor.
    ///
    /// May perform a copy
    fn into_tensor(self) -> Tensor;
}

/// Convenient conversion to Arc<Tensor>.
pub trait IntoArcTensor: Sized {
    /// Convert Self to a Arc<Tensor>.
    ///
    /// May perform a copy
    fn into_arc_tensor(self) -> Arc<Tensor>;
}

impl<D: ::ndarray::Dimension, T: Datum> IntoTensor for Array<T, D> {
    fn into_tensor(self) -> Tensor {
        Tensor::from(self)
    }
}

impl<D: ::ndarray::Dimension, T: Datum> IntoArcTensor for Array<T, D> {
    fn into_arc_tensor(self) -> Arc<Tensor> {
        Arc::new(Tensor::from(self))
    }
}

impl IntoTensor for Arc<Tensor> {
    fn into_tensor(self) -> Tensor {
        Arc::try_unwrap(self).unwrap_or_else(|t| (*t).clone())
    }
}

impl IntoArcTensor for Tensor {
    fn into_arc_tensor(self) -> Arc<Tensor> {
        Arc::new(self)
    }
}

impl IntoArcTensor for Arc<Tensor> {
    fn into_arc_tensor(self) -> Arc<Tensor> {
        self
    }
}
