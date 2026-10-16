# SPDX-License-Identifier: GPL-3.0-or-later
def bad_value(value):
    raise ValueError("boom value")


class FilterModule(object):
    def filters(self):
        return {"bad_value": bad_value}
