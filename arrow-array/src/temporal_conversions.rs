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

//! Conversion methods for dates and times.

use crate::ArrowPrimitiveType;
use arrow_schema::{DataType, TimeUnit};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

/// Number of seconds in a day
pub const SECONDS_IN_DAY: i64 = 86_400;
/// Number of milliseconds in a second
pub const MILLISECONDS: i64 = 1_000;
/// Number of microseconds in a second
pub const MICROSECONDS: i64 = 1_000_000;
/// Number of nanoseconds in a second
pub const NANOSECONDS: i64 = 1_000_000_000;

/// Number of milliseconds in a day
pub const MILLISECONDS_IN_DAY: i64 = SECONDS_IN_DAY * MILLISECONDS;
/// Number of days between 0001-01-01 and 1970-01-01
pub const EPOCH_DAYS_FROM_CE: i32 = 719_163;

/// converts a `i32` representing a `date32` to [`NaiveDateTime`]
#[inline]
pub fn date32_to_datetime(v: i32) -> NaiveDateTime {
    NaiveDateTime::from_timestamp(v as i64 * SECONDS_IN_DAY, 0)
}

/// converts a `i64` representing a `date64` to [`NaiveDateTime`]
#[inline]
pub fn date64_to_datetime(v: i64) -> NaiveDateTime {
    let (sec, milli_sec) = split_second(v, MILLISECONDS);

    NaiveDateTime::from_timestamp(
        // extract seconds from milliseconds
        sec,
        // discard extracted seconds and convert milliseconds to nanoseconds
        milli_sec * MICROSECONDS as u32,
    )
}

/// converts a `i32` representing a `time32(s)` to [`NaiveDateTime`]
#[inline]
pub fn time32s_to_time(v: i32) -> NaiveTime {
    NaiveTime::from_num_seconds_from_midnight(v as u32, 0)
}

/// converts a `i32` representing a `time32(ms)` to [`NaiveDateTime`]
#[inline]
pub fn time32ms_to_time(v: i32) -> NaiveTime {
    let v = v as i64;
    NaiveTime::from_num_seconds_from_midnight(
        // extract seconds from milliseconds
        (v / MILLISECONDS) as u32,
        // discard extracted seconds and convert milliseconds to
        // nanoseconds
        (v % MILLISECONDS * MICROSECONDS) as u32,
    )
}

/// converts a `i64` representing a `time64(us)` to [`NaiveDateTime`]
#[inline]
pub fn time64us_to_time(v: i64) -> NaiveTime {
    NaiveTime::from_num_seconds_from_midnight(
        // extract seconds from microseconds
        (v / MICROSECONDS) as u32,
        // discard extracted seconds and convert microseconds to
        // nanoseconds
        (v % MICROSECONDS * MILLISECONDS) as u32,
    )
}

/// converts a `i64` representing a `time64(ns)` to [`NaiveDateTime`]
#[inline]
pub fn time64ns_to_time(v: i64) -> NaiveTime {
    NaiveTime::from_num_seconds_from_midnight(
        // extract seconds from nanoseconds
        (v / NANOSECONDS) as u32,
        // discard extracted seconds
        (v % NANOSECONDS) as u32,
    )
}

/// converts a `i64` representing a `timestamp(s)` to [`NaiveDateTime`]
#[inline]
pub fn timestamp_s_to_datetime(v: i64) -> NaiveDateTime {
    NaiveDateTime::from_timestamp(v, 0)
}

/// converts a `i64` representing a `timestamp(ms)` to [`NaiveDateTime`]
#[inline]
pub fn timestamp_ms_to_datetime(v: i64) -> NaiveDateTime {
    let (sec, milli_sec) = split_second(v, MILLISECONDS);

    NaiveDateTime::from_timestamp(
        // extract seconds from milliseconds
        sec,
        // discard extracted seconds and convert milliseconds to nanoseconds
        milli_sec * MICROSECONDS as u32,
    )
}

/// converts a `i64` representing a `timestamp(us)` to [`NaiveDateTime`]
#[inline]
pub fn timestamp_us_to_datetime(v: i64) -> NaiveDateTime {
    let (sec, micro_sec) = split_second(v, MICROSECONDS);

    NaiveDateTime::from_timestamp(
        // extract seconds from microseconds
        sec,
        // discard extracted seconds and convert microseconds to nanoseconds
        micro_sec * MILLISECONDS as u32,
    )
}

/// converts a `i64` representing a `timestamp(ns)` to [`NaiveDateTime`]
#[inline]
pub fn timestamp_ns_to_datetime(v: i64) -> NaiveDateTime {
    let (sec, nano_sec) = split_second(v, NANOSECONDS);

    NaiveDateTime::from_timestamp(
        // extract seconds from nanoseconds
        sec, // discard extracted seconds
        nano_sec,
    )
}

#[inline]
pub(crate) fn split_second(v: i64, base: i64) -> (i64, u32) {
    (v.div_euclid(base), v.rem_euclid(base) as u32)
}

/// converts a `i64` representing a `duration(s)` to [`Duration`]
#[inline]
pub fn duration_s_to_duration(v: i64) -> Duration {
    Duration::seconds(v)
}

/// converts a `i64` representing a `duration(ms)` to [`Duration`]
#[inline]
pub fn duration_ms_to_duration(v: i64) -> Duration {
    Duration::milliseconds(v)
}

/// converts a `i64` representing a `duration(us)` to [`Duration`]
#[inline]
pub fn duration_us_to_duration(v: i64) -> Duration {
    Duration::microseconds(v)
}

/// converts a `i64` representing a `duration(ns)` to [`Duration`]
#[inline]
pub fn duration_ns_to_duration(v: i64) -> Duration {
    Duration::nanoseconds(v)
}

/// Converts an [`ArrowPrimitiveType`] to [`NaiveDateTime`]
pub fn as_datetime<T: ArrowPrimitiveType>(v: i64) -> Option<NaiveDateTime> {
    match T::DATA_TYPE {
        DataType::Date32 => Some(date32_to_datetime(v as i32)),
        DataType::Date64 => Some(date64_to_datetime(v)),
        DataType::Time32(_) | DataType::Time64(_) => None,
        DataType::Timestamp(unit, _) => match unit {
            TimeUnit::Second => Some(timestamp_s_to_datetime(v)),
            TimeUnit::Millisecond => Some(timestamp_ms_to_datetime(v)),
            TimeUnit::Microsecond => Some(timestamp_us_to_datetime(v)),
            TimeUnit::Nanosecond => Some(timestamp_ns_to_datetime(v)),
        },
        // interval is not yet fully documented [ARROW-3097]
        DataType::Interval(_) => None,
        _ => None,
    }
}

/// Converts an [`ArrowPrimitiveType`] to [`NaiveDate`]
pub fn as_date<T: ArrowPrimitiveType>(v: i64) -> Option<NaiveDate> {
    as_datetime::<T>(v).map(|datetime| datetime.date())
}

/// Converts an [`ArrowPrimitiveType`] to [`NaiveTime`]
pub fn as_time<T: ArrowPrimitiveType>(v: i64) -> Option<NaiveTime> {
    match T::DATA_TYPE {
        DataType::Time32(unit) => {
            // safe to immediately cast to u32 as `self.value(i)` is positive i32
            let v = v as u32;
            match unit {
                TimeUnit::Second => Some(time32s_to_time(v as i32)),
                TimeUnit::Millisecond => Some(time32ms_to_time(v as i32)),
                _ => None,
            }
        }
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => Some(time64us_to_time(v)),
            TimeUnit::Nanosecond => Some(time64ns_to_time(v)),
            _ => None,
        },
        DataType::Timestamp(_, _) => as_datetime::<T>(v).map(|datetime| datetime.time()),
        DataType::Date32 | DataType::Date64 => Some(NaiveTime::from_hms(0, 0, 0)),
        DataType::Interval(_) => None,
        _ => None,
    }
}

/// Converts an [`ArrowPrimitiveType`] to [`Duration`]
pub fn as_duration<T: ArrowPrimitiveType>(v: i64) -> Option<Duration> {
    match T::DATA_TYPE {
        DataType::Duration(unit) => match unit {
            TimeUnit::Second => Some(duration_s_to_duration(v)),
            TimeUnit::Millisecond => Some(duration_ms_to_duration(v)),
            TimeUnit::Microsecond => Some(duration_us_to_duration(v)),
            TimeUnit::Nanosecond => Some(duration_ns_to_duration(v)),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::temporal_conversions::{
        date64_to_datetime, split_second, timestamp_ms_to_datetime,
        timestamp_ns_to_datetime, timestamp_us_to_datetime, NANOSECONDS,
    };
    use chrono::NaiveDateTime;

    #[test]
    fn negative_input_timestamp_ns_to_datetime() {
        assert_eq!(
            timestamp_ns_to_datetime(-1),
            NaiveDateTime::from_timestamp(-1, 999_999_999)
        );

        assert_eq!(
            timestamp_ns_to_datetime(-1_000_000_001),
            NaiveDateTime::from_timestamp(-2, 999_999_999)
        );
    }

    #[test]
    fn negative_input_timestamp_us_to_datetime() {
        assert_eq!(
            timestamp_us_to_datetime(-1),
            NaiveDateTime::from_timestamp(-1, 999_999_000)
        );

        assert_eq!(
            timestamp_us_to_datetime(-1_000_001),
            NaiveDateTime::from_timestamp(-2, 999_999_000)
        );
    }

    #[test]
    fn negative_input_timestamp_ms_to_datetime() {
        assert_eq!(
            timestamp_ms_to_datetime(-1),
            NaiveDateTime::from_timestamp(-1, 999_000_000)
        );

        assert_eq!(
            timestamp_ms_to_datetime(-1_001),
            NaiveDateTime::from_timestamp(-2, 999_000_000)
        );
    }

    #[test]
    fn negative_input_date64_to_datetime() {
        assert_eq!(
            date64_to_datetime(-1),
            NaiveDateTime::from_timestamp(-1, 999_000_000)
        );

        assert_eq!(
            date64_to_datetime(-1_001),
            NaiveDateTime::from_timestamp(-2, 999_000_000)
        );
    }

    #[test]
    fn test_split_seconds() {
        let (sec, nano_sec) = split_second(100, NANOSECONDS);
        assert_eq!(sec, 0);
        assert_eq!(nano_sec, 100);

        let (sec, nano_sec) = split_second(123_000_000_456, NANOSECONDS);
        assert_eq!(sec, 123);
        assert_eq!(nano_sec, 456);

        let (sec, nano_sec) = split_second(-1, NANOSECONDS);
        assert_eq!(sec, -1);
        assert_eq!(nano_sec, 999_999_999);

        let (sec, nano_sec) = split_second(-123_000_000_001, NANOSECONDS);
        assert_eq!(sec, -124);
        assert_eq!(nano_sec, 999_999_999);
    }
}
