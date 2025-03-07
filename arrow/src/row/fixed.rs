// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use crate::array::PrimitiveArray;
use crate::compute::SortOptions;
use crate::datatypes::ArrowPrimitiveType;
use crate::row::{null_sentinel, Rows};
use arrow_array::types::DecimalType;
use arrow_array::{BooleanArray, DecimalArray};
use arrow_buffer::{bit_util, MutableBuffer, ToByteSlice};
use arrow_data::{ArrayData, ArrayDataBuilder};
use arrow_schema::DataType;
use half::f16;

pub trait FromSlice {
    fn from_slice(slice: &[u8], invert: bool) -> Self;
}

impl<const N: usize> FromSlice for [u8; N] {
    #[inline]
    fn from_slice(slice: &[u8], invert: bool) -> Self {
        let mut t: Self = slice.try_into().unwrap();
        if invert {
            t.iter_mut().for_each(|o| *o = !*o);
        }
        t
    }
}

/// Encodes a value of a particular fixed width type into bytes according to the rules
/// described on [`super::RowConverter`]
pub trait FixedLengthEncoding: Copy {
    const ENCODED_LEN: usize = 1 + std::mem::size_of::<Self::Encoded>();

    type Encoded: Sized + Copy + FromSlice + AsRef<[u8]> + AsMut<[u8]>;

    fn encode(self) -> Self::Encoded;

    fn decode(encoded: Self::Encoded) -> Self;
}

impl FixedLengthEncoding for bool {
    type Encoded = [u8; 1];

    fn encode(self) -> [u8; 1] {
        [self as u8]
    }

    fn decode(encoded: Self::Encoded) -> Self {
        encoded[0] != 0
    }
}

macro_rules! encode_signed {
    ($n:expr, $t:ty) => {
        impl FixedLengthEncoding for $t {
            type Encoded = [u8; $n];

            fn encode(self) -> [u8; $n] {
                let mut b = self.to_be_bytes();
                // Toggle top "sign" bit to ensure consistent sort order
                b[0] ^= 0x80;
                b
            }

            fn decode(mut encoded: Self::Encoded) -> Self {
                // Toggle top "sign" bit
                encoded[0] ^= 0x80;
                Self::from_be_bytes(encoded)
            }
        }
    };
}

encode_signed!(1, i8);
encode_signed!(2, i16);
encode_signed!(4, i32);
encode_signed!(8, i64);
encode_signed!(16, i128);

macro_rules! encode_unsigned {
    ($n:expr, $t:ty) => {
        impl FixedLengthEncoding for $t {
            type Encoded = [u8; $n];

            fn encode(self) -> [u8; $n] {
                self.to_be_bytes()
            }

            fn decode(encoded: Self::Encoded) -> Self {
                Self::from_be_bytes(encoded)
            }
        }
    };
}

encode_unsigned!(1, u8);
encode_unsigned!(2, u16);
encode_unsigned!(4, u32);
encode_unsigned!(8, u64);

impl FixedLengthEncoding for f16 {
    type Encoded = [u8; 2];

    fn encode(self) -> [u8; 2] {
        // https://github.com/rust-lang/rust/blob/9c20b2a8cc7588decb6de25ac6a7912dcef24d65/library/core/src/num/f32.rs#L1176-L1260
        let s = self.to_bits() as i16;
        let val = s ^ (((s >> 15) as u16) >> 1) as i16;
        val.encode()
    }

    fn decode(encoded: Self::Encoded) -> Self {
        let bits = i16::decode(encoded);
        let val = bits ^ (((bits >> 15) as u16) >> 1) as i16;
        Self::from_bits(val as u16)
    }
}

impl FixedLengthEncoding for f32 {
    type Encoded = [u8; 4];

    fn encode(self) -> [u8; 4] {
        // https://github.com/rust-lang/rust/blob/9c20b2a8cc7588decb6de25ac6a7912dcef24d65/library/core/src/num/f32.rs#L1176-L1260
        let s = self.to_bits() as i32;
        let val = s ^ (((s >> 31) as u32) >> 1) as i32;
        val.encode()
    }

    fn decode(encoded: Self::Encoded) -> Self {
        let bits = i32::decode(encoded);
        let val = bits ^ (((bits >> 31) as u32) >> 1) as i32;
        Self::from_bits(val as u32)
    }
}

impl FixedLengthEncoding for f64 {
    type Encoded = [u8; 8];

    fn encode(self) -> [u8; 8] {
        // https://github.com/rust-lang/rust/blob/9c20b2a8cc7588decb6de25ac6a7912dcef24d65/library/core/src/num/f32.rs#L1176-L1260
        let s = self.to_bits() as i64;
        let val = s ^ (((s >> 63) as u64) >> 1) as i64;
        val.encode()
    }

    fn decode(encoded: Self::Encoded) -> Self {
        let bits = i64::decode(encoded);
        let val = bits ^ (((bits >> 63) as u64) >> 1) as i64;
        Self::from_bits(val as u64)
    }
}

pub type RawDecimal128 = RawDecimal<16>;
pub type RawDecimal256 = RawDecimal<32>;

/// The raw bytes of a decimal
#[derive(Copy, Clone)]
pub struct RawDecimal<const N: usize>(pub [u8; N]);

impl<const N: usize> ToByteSlice for RawDecimal<N> {
    fn to_byte_slice(&self) -> &[u8] {
        &self.0
    }
}

impl<const N: usize> FixedLengthEncoding for RawDecimal<N> {
    type Encoded = [u8; N];

    fn encode(self) -> [u8; N] {
        let mut val = self.0;
        // Convert to big endian representation
        val.reverse();
        // Toggle top "sign" bit to ensure consistent sort order
        val[0] ^= 0x80;
        val
    }

    fn decode(mut encoded: Self::Encoded) -> Self {
        encoded[0] ^= 0x80;
        encoded.reverse();
        Self(encoded)
    }
}

/// Returns the total encoded length (including null byte) for a value of type `T::Native`
pub const fn encoded_len<T>(_col: &PrimitiveArray<T>) -> usize
where
    T: ArrowPrimitiveType,
    T::Native: FixedLengthEncoding,
{
    T::Native::ENCODED_LEN
}

/// Fixed width types are encoded as
///
/// - 1 byte `0` if null or `1` if valid
/// - bytes of [`FixedLengthEncoding`]
pub fn encode<T: FixedLengthEncoding, I: IntoIterator<Item = Option<T>>>(
    out: &mut Rows,
    i: I,
    opts: SortOptions,
) {
    for (offset, maybe_val) in out.offsets.iter_mut().skip(1).zip(i) {
        let end_offset = *offset + T::ENCODED_LEN;
        if let Some(val) = maybe_val {
            let to_write = &mut out.buffer[*offset..end_offset];
            to_write[0] = 1;
            let mut encoded = val.encode();
            if opts.descending {
                // Flip bits to reverse order
                encoded.as_mut().iter_mut().for_each(|v| *v = !*v)
            }
            to_write[1..].copy_from_slice(encoded.as_ref())
        } else {
            out.buffer[*offset] = null_sentinel(opts);
        }
        *offset = end_offset;
    }
}

/// Splits `len` bytes from `src`
#[inline]
fn split_off<'a>(src: &mut &'a [u8], len: usize) -> &'a [u8] {
    let v = &src[..len];
    *src = &src[len..];
    v
}

/// Decodes a `BooleanArray` from rows
pub fn decode_bool(rows: &mut [&[u8]], options: SortOptions) -> BooleanArray {
    let true_val = match options.descending {
        true => !1,
        false => 1,
    };

    let len = rows.len();

    let mut null_count = 0;
    let mut nulls = MutableBuffer::new(bit_util::ceil(len, 64) * 8);
    let mut values = MutableBuffer::new(bit_util::ceil(len, 64) * 8);

    let chunks = len / 64;
    let remainder = len % 64;
    for chunk in 0..chunks {
        let mut null_packed = 0;
        let mut values_packed = 0;

        for bit_idx in 0..64 {
            let i = split_off(&mut rows[bit_idx + chunk * 64], 2);
            let (null, value) = (i[0] == 1, i[1] == true_val);
            null_count += !null as usize;
            null_packed |= (null as u64) << bit_idx;
            values_packed |= (value as u64) << bit_idx;
        }

        nulls.push(null_packed);
        values.push(values_packed);
    }

    if remainder != 0 {
        let mut null_packed = 0;
        let mut values_packed = 0;

        for bit_idx in 0..remainder {
            let i = split_off(&mut rows[bit_idx + chunks * 64], 2);
            let (null, value) = (i[0] == 1, i[1] == true_val);
            null_count += !null as usize;
            null_packed |= (null as u64) << bit_idx;
            values_packed |= (value as u64) << bit_idx;
        }

        nulls.push(null_packed);
        values.push(values_packed);
    }

    let builder = ArrayDataBuilder::new(DataType::Boolean)
        .len(rows.len())
        .null_count(null_count)
        .add_buffer(values.into())
        .null_bit_buffer(Some(nulls.into()));

    // SAFETY:
    // Buffers are the correct length
    unsafe { BooleanArray::from(builder.build_unchecked()) }
}

/// Decodes a `ArrayData` from rows based on the provided `FixedLengthEncoding` `T`
fn decode_fixed<T: FixedLengthEncoding + ToByteSlice>(
    rows: &mut [&[u8]],
    data_type: DataType,
    options: SortOptions,
) -> ArrayData {
    let len = rows.len();

    let mut null_count = 0;
    let mut nulls = MutableBuffer::new(bit_util::ceil(len, 64) * 8);
    let mut values = MutableBuffer::new(std::mem::size_of::<T>() * len);

    let chunks = len / 64;
    let remainder = len % 64;
    for chunk in 0..chunks {
        let mut null_packed = 0;

        for bit_idx in 0..64 {
            let i = split_off(&mut rows[bit_idx + chunk * 64], T::ENCODED_LEN);
            let null = i[0] == 1;
            null_count += !null as usize;
            null_packed |= (null as u64) << bit_idx;

            let value = T::Encoded::from_slice(&i[1..], options.descending);
            values.push(T::decode(value));
        }

        nulls.push(null_packed);
    }

    if remainder != 0 {
        let mut null_packed = 0;

        for bit_idx in 0..remainder {
            let i = split_off(&mut rows[bit_idx + chunks * 64], T::ENCODED_LEN);
            let null = i[0] == 1;
            null_count += !null as usize;
            null_packed |= (null as u64) << bit_idx;

            let value = T::Encoded::from_slice(&i[1..], options.descending);
            values.push(T::decode(value));
        }

        nulls.push(null_packed);
    }

    let builder = ArrayDataBuilder::new(data_type)
        .len(rows.len())
        .null_count(null_count)
        .add_buffer(values.into())
        .null_bit_buffer(Some(nulls.into()));

    // SAFETY: Buffers correct length
    unsafe { builder.build_unchecked() }
}

/// Decodes a `DecimalArray` from rows
pub fn decode_decimal<const N: usize, T: DecimalType>(
    rows: &mut [&[u8]],
    options: SortOptions,
    precision: u8,
    scale: u8,
) -> DecimalArray<T> {
    decode_fixed::<RawDecimal<N>>(rows, T::TYPE_CONSTRUCTOR(precision, scale), options)
        .into()
}

/// Decodes a `PrimitiveArray` from rows
pub fn decode_primitive<T: ArrowPrimitiveType>(
    rows: &mut [&[u8]],
    options: SortOptions,
) -> PrimitiveArray<T>
where
    T::Native: FixedLengthEncoding + ToByteSlice,
{
    decode_fixed::<T::Native>(rows, T::DATA_TYPE, options).into()
}
