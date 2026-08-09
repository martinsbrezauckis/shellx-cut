from __future__ import annotations

import math
from decimal import Decimal, InvalidOperation


def finite_float(value, default=None) -> float:
    try:
        dec = Decimal(str(value).strip())
    except (InvalidOperation, ValueError):
        if default is not None:
            return default
        raise ValueError(f"not a number: {value!r}")
    if not dec.is_finite():
        if default is not None:
            return default
        raise ValueError(f"non-finite number: {value!r}")
    return dec.__float__()


def finite_number(value, default=0.0) -> float:
    try:
        if isinstance(value, str):
            return finite_float(value, default)
        if hasattr(value, "item"):
            value = value.item()
        if hasattr(value, "__float__"):
            out = value.__float__()
        else:
            out = finite_float(value, default)
    except (TypeError, ValueError, InvalidOperation):
        if default is not None:
            return default
        raise
    if not math.isfinite(out):
        if default is not None:
            return default
        raise ValueError(f"non-finite number: {value!r}")
    return out
