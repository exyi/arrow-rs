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

use crate::compute::SortOptions;
use crate::row::fixed::{FixedLengthEncoding, FromSlice, RawDecimal};
use crate::row::interner::{Interned, OrderPreservingInterner};
use crate::row::{null_sentinel, Rows};
use arrow_array::builder::*;
use arrow_array::cast::*;
use arrow_array::types::*;
use arrow_array::*;
use arrow_buffer::{ArrowNativeType, MutableBuffer, ToByteSlice};
use arrow_data::{ArrayData, ArrayDataBuilder};
use arrow_schema::{ArrowError, DataType, IntervalUnit, TimeUnit};
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// Computes the dictionary mapping for the given dictionary values
pub fn compute_dictionary_mapping(
    interner: &mut OrderPreservingInterner,
    values: &ArrayRef,
) -> Result<Vec<Option<Interned>>, ArrowError> {
    Ok(downcast_primitive_array! {
        values => interner
            .intern(values.iter().map(|x| x.map(|x| x.encode()))),
        DataType::Binary => {
            let iter = as_generic_binary_array::<i64>(values).iter();
            interner.intern(iter)
        }
        DataType::LargeBinary => {
            let iter = as_generic_binary_array::<i64>(values).iter();
            interner.intern(iter)
        }
        DataType::Utf8 => {
            let iter = as_string_array(values).iter().map(|x| x.map(|x| x.as_bytes()));
            interner.intern(iter)
        }
        DataType::LargeUtf8 => {
            let iter = as_largestring_array(values).iter().map(|x| x.map(|x| x.as_bytes()));
            interner.intern(iter)
        }
        t => return Err(ArrowError::NotYetImplemented(format!("dictionary value {} is not supported", t))),
    })
}

/// Dictionary types are encoded as
///
/// - single `0_u8` if null
/// - the bytes of the corresponding normalized key including the null terminator
pub fn encode_dictionary<K: ArrowDictionaryKeyType>(
    out: &mut Rows,
    column: &DictionaryArray<K>,
    normalized_keys: &[Option<&[u8]>],
    opts: SortOptions,
) {
    for (offset, k) in out.offsets.iter_mut().skip(1).zip(column.keys()) {
        match k.and_then(|k| normalized_keys[k.as_usize()]) {
            Some(normalized_key) => {
                let end_offset = *offset + 1 + normalized_key.len();
                out.buffer[*offset] = 1;
                out.buffer[*offset + 1..end_offset].copy_from_slice(normalized_key);
                // Negate if descending
                if opts.descending {
                    out.buffer[*offset..end_offset]
                        .iter_mut()
                        .for_each(|v| *v = !*v)
                }
                *offset = end_offset;
            }
            None => {
                out.buffer[*offset] = null_sentinel(opts);
                *offset += 1;
            }
        }
    }
}

/// Decodes a string array from `rows` with the provided `options`
///
/// # Safety
///
/// `interner` must contain valid data for the provided `value_type`
pub unsafe fn decode_dictionary<K: ArrowDictionaryKeyType>(
    interner: &OrderPreservingInterner,
    value_type: &DataType,
    options: SortOptions,
    rows: &mut [&[u8]],
) -> Result<DictionaryArray<K>, ArrowError> {
    let len = rows.len();
    let mut dictionary: HashMap<Interned, K::Native> = HashMap::with_capacity(len);

    let null_sentinel = null_sentinel(options);

    // If descending, the null terminator will have been negated
    let null_terminator = match options.descending {
        true => 0xFF,
        false => 0_u8,
    };

    let mut null_builder = BooleanBufferBuilder::new(len);
    let mut keys = BufferBuilder::<K::Native>::new(len);
    let mut values = Vec::with_capacity(len);
    let mut null_count = 0;
    let mut key_scratch = Vec::new();

    for row in rows {
        if row[0] == null_sentinel {
            null_builder.append(false);
            null_count += 1;
            *row = &row[1..];
            keys.append(K::Native::default());
            continue;
        }

        let key_offset = row
            .iter()
            .skip(1)
            .position(|x| *x == null_terminator)
            .unwrap();

        // Extract the normalized key including the null terminator
        let key = &row[1..key_offset + 2];
        *row = &row[key_offset + 2..];

        let interned = match options.descending {
            true => {
                // If options.descending the normalized key will have been
                // negated we must first reverse this
                key_scratch.clear();
                key_scratch.extend_from_slice(key);
                key_scratch.iter_mut().for_each(|o| *o = !*o);
                interner.lookup(&key_scratch).unwrap()
            }
            false => interner.lookup(key).unwrap(),
        };

        let k = match dictionary.entry(interned) {
            Entry::Vacant(v) => {
                let k = values.len();
                values.push(interner.value(interned));
                let key = K::Native::from_usize(k)
                    .ok_or(ArrowError::DictionaryKeyOverflowError)?;
                *v.insert(key)
            }
            Entry::Occupied(o) => *o.get(),
        };

        keys.append(k);
        null_builder.append(true);
    }

    let child = match &value_type {
        DataType::Null => NullArray::new(values.len()).into_data(),
        DataType::Boolean => decode_bool(&values),
        DataType::Int8 => decode_primitive::<Int8Type>(&values),
        DataType::Int16 => decode_primitive::<Int16Type>(&values),
        DataType::Int32 => decode_primitive::<Int32Type>(&values),
        DataType::Int64 => decode_primitive::<Int64Type>(&values),
        DataType::UInt8 => decode_primitive::<UInt8Type>(&values),
        DataType::UInt16 => decode_primitive::<UInt16Type>(&values),
        DataType::UInt32 => decode_primitive::<UInt32Type>(&values),
        DataType::UInt64 => decode_primitive::<UInt64Type>(&values),
        DataType::Float16 => decode_primitive::<Float16Type>(&values),
        DataType::Float32 => decode_primitive::<Float32Type>(&values),
        DataType::Float64 => decode_primitive::<Float64Type>(&values),
        DataType::Timestamp(TimeUnit::Second, _) => {
            decode_primitive::<TimestampSecondType>(&values)
        }
        DataType::Timestamp(TimeUnit::Millisecond, _) => {
            decode_primitive::<TimestampMillisecondType>(&values)
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            decode_primitive::<TimestampMicrosecondType>(&values)
        }
        DataType::Timestamp(TimeUnit::Nanosecond, _) => {
            decode_primitive::<TimestampNanosecondType>(&values)
        }
        DataType::Date32 => decode_primitive::<Date32Type>(&values),
        DataType::Date64 => decode_primitive::<Date64Type>(&values),
        DataType::Time32(t) => match t {
            TimeUnit::Second => decode_primitive::<Time32SecondType>(&values),
            TimeUnit::Millisecond => decode_primitive::<Time32MillisecondType>(&values),
            _ => unreachable!(),
        },
        DataType::Time64(t) => match t {
            TimeUnit::Microsecond => decode_primitive::<Time64MicrosecondType>(&values),
            TimeUnit::Nanosecond => decode_primitive::<Time64NanosecondType>(&values),
            _ => unreachable!(),
        },
        DataType::Duration(TimeUnit::Second) => {
            decode_primitive::<DurationSecondType>(&values)
        }
        DataType::Duration(TimeUnit::Millisecond) => {
            decode_primitive::<DurationMillisecondType>(&values)
        }
        DataType::Duration(TimeUnit::Microsecond) => {
            decode_primitive::<DurationMicrosecondType>(&values)
        }
        DataType::Duration(TimeUnit::Nanosecond) => {
            decode_primitive::<DurationNanosecondType>(&values)
        }
        DataType::Interval(IntervalUnit::DayTime) => {
            decode_primitive::<IntervalDayTimeType>(&values)
        }
        DataType::Interval(IntervalUnit::MonthDayNano) => {
            decode_primitive::<IntervalMonthDayNanoType>(&values)
        }
        DataType::Interval(IntervalUnit::YearMonth) => {
            decode_primitive::<IntervalYearMonthType>(&values)
        }
        DataType::Decimal128(p, s) => {
            decode_decimal::<16, Decimal128Type>(&values, *p, *s)
        }
        DataType::Decimal256(p, s) => {
            decode_decimal::<32, Decimal256Type>(&values, *p, *s)
        }
        DataType::Utf8 => decode_string::<i32>(&values),
        DataType::LargeUtf8 => decode_string::<i64>(&values),
        DataType::Binary => decode_binary::<i32>(&values),
        DataType::LargeBinary => decode_binary::<i64>(&values),
        _ => {
            return Err(ArrowError::NotYetImplemented(format!(
                "decoding dictionary values of {}",
                value_type
            )))
        }
    };

    let data_type =
        DataType::Dictionary(Box::new(K::DATA_TYPE), Box::new(value_type.clone()));

    let builder = ArrayDataBuilder::new(data_type)
        .len(len)
        .null_bit_buffer(Some(null_builder.finish()))
        .null_count(null_count)
        .add_buffer(keys.finish())
        .add_child_data(child);

    Ok(DictionaryArray::from(builder.build_unchecked()))
}

/// Decodes a binary array from dictionary values
///
/// # Safety
///
/// Values must be valid UTF-8
fn decode_binary<O: OffsetSizeTrait>(values: &[&[u8]]) -> ArrayData {
    let capacity = values.iter().map(|x| x.len()).sum();
    let mut builder = GenericBinaryBuilder::<O>::with_capacity(values.len(), capacity);
    for v in values {
        builder.append_value(v)
    }
    builder.finish().into_data()
}

/// Decodes a string array from dictionary values
///
/// # Safety
///
/// Values must be valid UTF-8
unsafe fn decode_string<O: OffsetSizeTrait>(values: &[&[u8]]) -> ArrayData {
    let d = match O::IS_LARGE {
        true => DataType::LargeUtf8,
        false => DataType::Utf8,
    };

    decode_binary::<O>(values)
        .into_builder()
        .data_type(d)
        .build_unchecked()
}

/// Decodes a boolean array from dictionary values
fn decode_bool(values: &[&[u8]]) -> ArrayData {
    let mut builder = BooleanBufferBuilder::new(values.len());
    for value in values {
        builder.append(bool::decode([value[0]]))
    }

    let builder = ArrayDataBuilder::new(DataType::Boolean)
        .len(values.len())
        .add_buffer(builder.finish());

    // SAFETY: Buffers correct length
    unsafe { builder.build_unchecked() }
}

/// Decodes a fixed length type array from dictionary values
fn decode_fixed<T: FixedLengthEncoding + ToByteSlice>(
    values: &[&[u8]],
    data_type: DataType,
) -> ArrayData {
    let mut buffer = MutableBuffer::new(std::mem::size_of::<T>() * values.len());

    for value in values {
        let value = T::Encoded::from_slice(value, false);
        buffer.push(T::decode(value))
    }

    let builder = ArrayDataBuilder::new(data_type)
        .len(values.len())
        .add_buffer(buffer.into());

    // SAFETY: Buffers correct length
    unsafe { builder.build_unchecked() }
}

/// Decodes a `PrimitiveArray` from dictionary values
fn decode_primitive<T: ArrowPrimitiveType>(values: &[&[u8]]) -> ArrayData
where
    T::Native: FixedLengthEncoding,
{
    decode_fixed::<T::Native>(values, T::DATA_TYPE)
}

/// Decodes a `DecimalArray` from dictionary values
fn decode_decimal<const N: usize, T: DecimalType>(
    values: &[&[u8]],
    precision: u8,
    scale: u8,
) -> ArrayData {
    decode_fixed::<RawDecimal<N>>(values, T::TYPE_CONSTRUCTOR(precision, scale))
}
