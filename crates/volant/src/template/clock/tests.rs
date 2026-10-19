// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// What `datetime.strftime` printed on Python 3.13 with glibc, for a `datetime` with no zone. The
/// last two columns are the format and its text.
const STRFTIME: &[(i64, u32, &str, &str)] = &[
    (
        1000000000,
        123456,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Sun|Sunday|Sep|September|Sun Sep  9 01:46:40 2001|20|09|09/09/01| 9|2001-09-09|01|2001|Sep|01|01|252| 1| 1|09|46|AM|am|01:46:40 AM|01:46|40|01:46:40|7|36|36|0|36|09/09/01|01:46:40|01|2001|||%|123456",
    ),
    (
        1000000000,
        123456,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "9/9/01| 9|09|SUN|am|0000002001|009|1| 1|252|SEPTEMBER|SUN|1AM|20010909|014640123456|\n|\t|%Q|09/09/01|01|    %|%",
    ),
    (
        1709251199,
        500000,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Thu|Thursday|Feb|February|Thu Feb 29 23:59:59 2024|20|29|02/29/24|29|2024-02-29|24|2024|Feb|23|11|060|23|11|02|59|PM|pm|11:59:59 PM|23:59|59|23:59:59|4|08|09|4|09|02/29/24|23:59:59|24|2024|||%|500000",
    ),
    (
        1709251199,
        500000,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "29/2/24|29|29|THU|pm|0000002024|029|23|23|60|FEBRUARY|THU|11PM|20240229|235959500000|\n|\t|%Q|02/29/24|24|    %|%",
    ),
    (
        1609632000,
        0,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Sun|Sunday|Jan|January|Sun Jan  3 00:00:00 2021|20|03|01/03/21| 3|2021-01-03|20|2020|Jan|00|12|003| 0|12|01|00|AM|am|12:00:00 AM|00:00|00|00:00:00|7|01|53|0|00|01/03/21|00:00:00|21|2021|||%|000000",
    ),
    (
        1609632000,
        0,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "3/1/21| 3|03|SUN|am|0000002021|003|0| 0|3|JANUARY|SUN|12AM|20210103|000000000000|\n|\t|%Q|01/03/21|21|    %|%",
    ),
    (
        1735516800,
        7,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Mon|Monday|Dec|December|Mon Dec 30 00:00:00 2024|20|30|12/30/24|30|2024-12-30|25|2025|Dec|00|12|365| 0|12|12|00|AM|am|12:00:00 AM|00:00|00|00:00:00|1|52|01|1|53|12/30/24|00:00:00|24|2024|||%|000007",
    ),
    (
        1735516800,
        7,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "30/12/24|30|30|MON|am|0000002024|030|0| 0|365|DECEMBER|MON|12AM|20241230|000000000007|\n|\t|%Q|12/30/24|24|    %|%",
    ),
    (
        0,
        0,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Thu|Thursday|Jan|January|Thu Jan  1 00:00:00 1970|19|01|01/01/70| 1|1970-01-01|70|1970|Jan|00|12|001| 0|12|01|00|AM|am|12:00:00 AM|00:00|00|00:00:00|4|00|01|4|00|01/01/70|00:00:00|70|1970|||%|000000",
    ),
    (
        0,
        0,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "1/1/70| 1|01|THU|am|0000001970|001|0| 0|1|JANUARY|THU|12AM|19700101|000000000000|\n|\t|%Q|01/01/70|70|    %|%",
    ),
    (
        1792411200,
        999999,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Mon|Monday|Oct|October|Mon Oct 19 12:00:00 2026|20|19|10/19/26|19|2026-10-19|26|2026|Oct|12|12|292|12|12|10|00|PM|pm|12:00:00 PM|12:00|00|12:00:00|1|42|43|1|42|10/19/26|12:00:00|26|2026|||%|999999",
    ),
    (
        1792411200,
        999999,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "19/10/26|19|19|MON|pm|0000002026|019|12|12|292|OCTOBER|MON|12PM|20261019|120000999999|\n|\t|%Q|10/19/26|26|    %|%",
    ),
    (
        -14182940,
        0,
        "%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%p|%P|%r|%R|%S|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%|%f",
        "Sun|Sunday|Jul|July|Sun Jul 20 20:17:40 1969|19|20|07/20/69|20|1969-07-20|69|1969|Jul|20|08|201|20| 8|07|17|PM|pm|08:17:40 PM|20:17|40|20:17:40|7|29|29|0|28|07/20/69|20:17:40|69|1969|||%|000000",
    ),
    (
        -14182940,
        0,
        "%-d/%-m/%y|%_d|%0e|%^a|%#p|%10Y|%3d|%-H|%_H|%-j|%^B|%#a|%-I%p|%Y%m%d|%H%M%S%f|%n|%t|%Q|%Ex|%Oy|%5%|%",
        "20/7/69|20|20|SUN|pm|0000001969|020|20|20|201|JULY|SUN|8PM|19690720|201740000000|\n|\t|%Q|07/20/69|69|    %|%",
    ),
];

#[test]
fn strftime_prints_what_python_prints() {
    for &(secs, micros, format, want) in STRFTIME {
        let moment = DateTime::from_utc(secs, micros);
        assert_eq!(strftime::format(&moment, format), want, "{secs} {format}");
    }
}

/// Dates around the epoch, a leap day and the year 2000 (a leap year) come back from `days`.
#[test]
fn civil_dates_and_day_counts_are_inverse() {
    for days in (-800_000..800_000).step_by(37) {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(
            days_from_civil(year, month, day),
            days,
            "{year}-{month}-{day}"
        );
    }
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
}

#[test]
fn isoformat_writes_the_microseconds_only_when_there_are_some() {
    let whole = DateTime::from_utc(1_000_000_000, 0);
    let part = DateTime::from_utc(1_000_000_000, 5);
    assert_eq!(whole.isoformat('T', Timespec::Auto), "2001-09-09T01:46:40");
    assert_eq!(
        part.isoformat('T', Timespec::Auto),
        "2001-09-09T01:46:40.000005"
    );
    assert_eq!(part.python_str(), "2001-09-09 01:46:40.000005");
    assert_eq!(part.isoformat('_', Timespec::Hours), "2001-09-09_01");
    assert_eq!(part.isoformat('T', Timespec::Minutes), "2001-09-09T01:46");
    assert_eq!(
        part.isoformat('T', Timespec::Milliseconds),
        "2001-09-09T01:46:40.000"
    );
    assert_eq!(
        part.isoformat('T', Timespec::Seconds),
        "2001-09-09T01:46:40"
    );
}

/// Time order is field order, and the UTC seconds come back from the fields.
#[test]
fn moments_order_by_time() {
    let earlier = DateTime::from_utc(1_000_000_000, 999_999);
    let later = DateTime::from_utc(1_000_000_001, 0);
    assert!(earlier < later);
    assert_eq!(earlier.utc_seconds(), 1_000_000_000);
    assert_eq!(DateTime::from_utc(-1, 0).utc_seconds(), -1);
    assert_eq!(
        DateTime::from_utc(-1, 0).isoformat('T', Timespec::Auto),
        "1969-12-31T23:59:59"
    );
}
