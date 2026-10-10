#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn answer(source: &str) -> Result<Answer, String> {
    let program = compile(source)?;
    run(
        &program,
        &ExpressionContext::detached(),
        Slot::default(),
        &[],
    )
}

fn num(source: &str) -> f64 {
    match answer(source) {
        Ok(Answer::Number(n)) => n,
        other => panic!("{source}: {other:?}"),
    }
}

fn list(source: &str) -> Vec<f64> {
    match answer(source) {
        Ok(Answer::List(list)) => list,
        other => panic!("{source}: {other:?}"),
    }
}

fn text(source: &str) -> String {
    match answer(source) {
        Ok(Answer::Text(text)) => text,
        other => panic!("{source}: {other:?}"),
    }
}

/// The JavaScript an After Effects expression is written in runs as written:
/// statements that end at a line break, functions called above where they are
/// declared, and the last statement's value as the answer.
#[test]
fn the_language_an_expression_is_written_in_runs() {
    assert_eq!(num("var a = 2\rvar b = 3\ra * b"), 6.0);
    assert_eq!(num("twice(4); function twice(x) { return x * 2 }"), 8.0);
    assert_eq!(
        num("var n = 0; for (var i = 0; i < 5; i++) { if (i == 3) continue; n += i } n"),
        7.0
    );
    assert_eq!(num("var n = 10; while (n > 3) n -= 2; n"), 2.0);
    assert_eq!(num("var n = 0; do { n++ } while (n < 4); n"), 4.0);
    assert_eq!(num("if (1 > 2) { 10 } else { 20 }"), 20.0);
    // A name nobody declared is made by assigning to it.
    assert_eq!(num("x = 5; x > 3 ? x * 2 : 0"), 10.0);
    assert_eq!(num("const [a, , c] = [1, 2, 3]; a + c"), 4.0);
    assert_eq!(
        num("let total = 0; for (const n of [1, 2, 3]) total += n; total"),
        6.0
    );
    assert_eq!(
        num("var keys = ''; for (var k in {a: 1, b: 2}) keys += k; keys.length"),
        2.0
    );
    assert_eq!(
        num("var o = {a: 1, 'b': 2}; o.c = 3; o.a + o['b'] + o.c"),
        6.0
    );
    assert_eq!(
        num("[1, 2, 3, 4].filter(n => n % 2 == 0).map(function (n) { return n * 10 }).reduce((a, b) => a + b, 0)"),
        60.0
    );
    assert_eq!(
        num("var out = 0; try { missing() } catch (e) { out = 7 } finally { out += 1 } out"),
        8.0
    );
    assert_eq!(
        num("switch (2) { case 1: 10; break; case 2: 20; break; default: 30 }"),
        20.0
    );
    assert_eq!(
        num("function make() { var n = 0; return function () { n += 1; return n } } var f = make(); f(); f()"),
        2.0
    );
    assert_eq!(num("typeof nothing == 'undefined' ? 1 : 0"), 1.0);
    assert_eq!(
        num("var a = [3, 1, 2]; a.sort((x, y) => x - y); a[0] + a.indexOf(3)"),
        3.0
    );
    assert_eq!(num("var i = 1; var j = i++ + ++i; j * 2 ** 2"), 16.0);
    assert_eq!(
        num("(5 & 3) + (5 | 3) + (1 << 4) + (null ?? 2) + (0 || 4) + (1 && 8)"),
        38.0
    );
    assert_eq!(
        num("Math.max(1, Math.round(2.5), Math.floor(-0.5)) + Math.PI - Math.PI"),
        3.0
    );
    assert_eq!(
        num("parseInt('12px') + parseFloat('0.5em') + Number('2')"),
        14.5
    );
    assert_eq!(text("`${1 + 1} of ${'three'.toUpperCase()}`"), "2 of THREE");
    assert_eq!(
        text("'a,b'.split(',').reverse().join('-') + (7).toFixed(1) + 'x'.padStart(3, '.')"),
        "b-a7.0..x"
    );
    assert_eq!(
        text("'shake'.slice(1, 3) + 'shake'.indexOf('k') + 1.5"),
        "ha31.5"
    );
}

/// After Effects reads arithmetic on lists as arithmetic on each number, which
/// JavaScript proper does not, and every expression on a position relies on it.
#[test]
fn lists_add_and_scale_the_way_after_effects_has_them() {
    assert_eq!(list("[10, 20] + [1, 2]"), [11.0, 22.0]);
    assert_eq!(list("[10, 20] - [1, 2] * 2"), [8.0, 16.0]);
    // A bare number added to a list joins its first item.
    assert_eq!(list("[10, 20] / 2 + 1"), [6.0, 10.0]);
    assert_eq!(list("-[1, 2]"), [-1.0, -2.0]);
    assert_eq!(list("add([1, 2, 3], [1, 1])"), [2.0, 3.0, 3.0]);
    assert_eq!(num("length([3, 4]) + length([0, 0], [3, 4])"), 10.0);
    assert_eq!(list("linear(5, 0, 10, [0, 0], [100, 50])"), [50.0, 25.0]);
    assert_eq!(
        num("ease(0.5, 10, 20) + easeIn(0, 5, 9) + easeOut(1, 5, 9)"),
        29.0
    );
    assert_eq!(list("clamp([12, -3], 0, 10)"), [10.0, 0.0]);
    assert_eq!(list("mul(normalize([0, 5]), 2)"), [0.0, 2.0]);
    assert_eq!(
        num("dot([1, 2], [3, 4]) + cross([1, 0, 0], [0, 1, 0])[2]"),
        12.0
    );
    assert_eq!(
        num("Math.round(radiansToDegrees(Math.PI) + degreesToRadians(0))"),
        180.0
    );
    let round_trip = list("hslToRgb(rgbToHsl([0.25, 0.5, 0.75, 1]))");
    for (got, want) in round_trip.iter().zip([0.25, 0.5, 0.75, 1.0]) {
        assert!((got - want).abs() < 1e-9, "{round_trip:?}");
    }
}

/// An expression runs on a render thread for every frame, so one that never
/// ends, or builds something enormous, has to stop by itself.
#[test]
fn a_runaway_expression_stops() {
    assert!(answer("while (true) {}").is_err());
    // Running out of steps is not something a `catch` gets to swallow.
    assert!(answer("try { while (true) {} } catch (e) { 1 }").is_err());
    assert!(answer("function f() { return f() } f()").is_err());
    assert!(answer("var a = []; while (true) a.push(1)").is_err());
    assert!(answer("var s = 'x'; while (true) s += s").is_err());
    assert!(compile(&"(".repeat(5000)).is_err());
    assert!(compile(&format!("{}1{}", "[".repeat(5000), "]".repeat(5000))).is_err());
    // A list that holds itself is still let go when the run ends.
    assert_eq!(num("var a = []; a.push(a); a.length"), 1.0);

    for refused in ["new Date()", "1 +", "'open", "a = ", "{ 1", "/* open"] {
        assert!(compile(refused).is_err(), "{refused}");
    }
    assert!(answer("nonesuch + 1").is_err());
    assert!(answer("undefined.x").is_err());
    assert!(answer("var f = 3; f()").is_err());
}

/// Random numbers come from the seed and the time and nothing else, so a
/// frame draws the same ones in the preview, in the export and on its cache
/// key.
#[test]
fn random_numbers_are_the_same_every_time() {
    let draw = "seedRandom(4, true); [random(), random(10), random(5, 6)]";
    let first = list(draw);
    assert_eq!(first, list(draw));
    assert!(first[0] >= 0.0 && first[0] < 1.0 && first[2] >= 5.0 && first[2] < 6.0);
    assert_ne!(first[0], first[1] / 10.0, "each draw is a new number");
    assert_ne!(
        first,
        list("seedRandom(5, true); [random(), random(10), random(5, 6)]")
    );
    let bell = num("seedRandom(1, true); gaussRandom()");
    assert!(bell > -2.0 && bell < 3.0);
    // Perlin noise is nought on every whole number.
    assert_eq!(num("noise([2, 3])"), 0.0);
}

/// A slot survives the trip through a property's `extra`, and an empty one
/// leaves nothing behind.
#[test]
fn a_slot_is_kept_with_its_property() {
    let mut extra = serde_json::Map::new();
    let pair = Slot::new(&[960.0, 540.0], 1, 9);
    pair.write(&mut extra);
    assert_eq!(Slot::read(&extra), pair);
    assert_eq!(pair.own(), Some(540.0));

    let one = Slot::new(&[30.0], 0, 0);
    one.write(&mut extra);
    assert_eq!(Slot::read(&extra), one);

    Slot::default().write(&mut extra);
    assert!(extra.is_empty());
    assert_eq!(Slot::read(&extra).own(), None);
}
