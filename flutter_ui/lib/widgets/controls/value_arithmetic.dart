// What a value well makes of what was typed into it: a number, or a sum.

/// What a value well makes of what was typed into it: a number, or a sum
/// (Caddis A3). `(1920-100)*0.5` commits 910.
///
/// **In plain terms.** Half of what a person types into a size or a position
/// is arithmetic they did in their head first — half of this, that less the
/// margin, twice the frame. The well now does the sum instead, over `+ - * /`,
/// brackets and a leading minus, with multiplication binding tighter than
/// addition the way it does on paper.
///
/// A plain number is read by [num.tryParse] exactly as before, so nothing
/// about typing a number changes — including the forms the parser knows and
/// this one deliberately does not, such as `1e3`. Anything that is not a sum
/// this understands, and any division by zero, comes back null, which is the
/// answer a well already had for text it could not read: keep the value it
/// has, say nothing, punish nobody.
num? parseNumberField(String text) {
  final plain = num.tryParse(text.trim());
  if (plain != null) return plain;
  return _Arithmetic(text.replaceAll(' ', '')).parse();
}

/// A recursive descent over the six symbols a value well needs, which is the
/// whole grammar — no dependency for four small methods.
class _Arithmetic {
  _Arithmetic(this._src);
  final String _src;
  int _at = 0;

  num? parse() {
    final value = _sum();
    // Trailing rubbish (`3+4x`) is not a sum with a tail, it is a mistake; and
    // a division by zero arrives here as an infinity or a NaN.
    if (value == null || _at != _src.length || !value.isFinite) return null;
    return value;
  }

  double? _sum() {
    var left = _product();
    while (left != null && _at < _src.length) {
      final op = _src[_at];
      if (op != '+' && op != '-') break;
      _at++;
      final right = _product();
      if (right == null) return null;
      left = op == '+' ? left + right : left - right;
    }
    return left;
  }

  double? _product() {
    var left = _atom();
    while (left != null && _at < _src.length) {
      final op = _src[_at];
      if (op != '*' && op != '/') break;
      _at++;
      final right = _atom();
      if (right == null) return null;
      left = op == '*' ? left * right : left / right;
    }
    return left;
  }

  double? _atom() {
    if (_at >= _src.length) return null;
    final c = _src[_at];
    if (c == '-' || c == '+') {
      _at++;
      final v = _atom();
      return v == null ? null : (c == '-' ? -v : v);
    }
    if (c == '(') {
      _at++;
      final v = _sum();
      if (v == null || _at >= _src.length || _src[_at] != ')') return null;
      _at++;
      return v;
    }
    final start = _at;
    while (_at < _src.length && _isNumberChar(_src.codeUnitAt(_at))) {
      _at++;
    }
    return _at == start ? null : double.tryParse(_src.substring(start, _at));
  }

  /// A digit or the decimal point. `double.tryParse` says whether what they
  /// spell is actually a number — `1.2.3` is not, and comes back null.
  static bool _isNumberChar(int unit) =>
      (unit >= 0x30 && unit <= 0x39) || unit == 0x2e;
}
