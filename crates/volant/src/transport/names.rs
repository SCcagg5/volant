// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading a setting that a host can name in several variables.
//!
//! ansible-core 2.19.12 lists the variables of a setting in its plugin and lets the last one
//! it finds beat the others. A caller lists the names the other way round, so the first name set
//! is the one that applies. A variable that holds no value (null) counts as not set.

use serde_json::Value;

use crate::vars::{HostSettings, host_setting};

/// The value of the first of `names` the host sets.
pub(crate) fn first_set<'a, V: HostSettings + ?Sized>(
    vars: &'a V,
    names: &[&str],
) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|name| host_setting(vars, name).filter(|value| !value.is_null()))
}

/// A password as text. A number is its own digits, which is how an inventory line
/// `ansible_password=1234` reaches here; any other shape is no password.
pub(crate) fn secret_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
