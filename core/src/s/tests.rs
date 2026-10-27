use super::*;

fn out(src: &str) -> String {
    let o = run(src, "", BTreeMap::new(), &[], 5_000_000, 7);
    assert_eq!(o.err, None, "{src}");
    o.out
}

#[test]
fn the_line_and_output() {
    assert_eq!(out("hello world >"), "\n hello world");
    assert_eq!(out("a >_ b >_"), "ab");
    assert_eq!(out("2 3 + =~ >"), "\n 5");
    assert_eq!(out("0.1 0.2 + =~ >"), "\n 0.30000000000000004");
    assert_eq!(out("7 2 % =~ > -7 2 % =~ >"), "\n 1\n 1");
    assert_eq!(out("⟨w⟩ > ⟨+!⟩ > \\abc >"), "\n w\n +!\n abc");
    assert_eq!(out("world who@ {hello ~who~} >"), "\n hello world");
    assert_eq!(out("{a ~~ b} >"), "\n a ~ b");
}

#[test]
fn frames_from_the_spec() {
    // `a b c d 2 ( + =~ )` leaves `a b 7` for c = 3, d = 4
    assert_eq!(out("3 c# 4 d# a b c~ d~ 2 ( + =~ ) >"), "\n a b 7");
    assert_eq!(out("x y z ## >"), "\n x y z 3");
}

#[test]
fn conditions_and_suffixes() {
    assert_eq!(out("3 4 << yes? no! >"), "\n yes");
    assert_eq!(out("a b = same? differ! >"), "\n differ");
    assert_eq!(out("5 x# 2 x; x~ > x: x~ >"), "\n 7\n 6");
    assert_eq!(out("1 2 3 all` all~ >"), "\n 1 2 3");
}

#[test]
fn scripts_repeat_and_tail_calls() {
    assert_eq!(out("{hi >} s$ 3 s."), "\n hi\n hi\n hi");
    assert_eq!(out("{hi >} s$ 0 s."), "");
    assert_eq!(out("{n~ >_} p$ 3 p &&&"), "012");
    // a loop by tail call: 200 000 rounds in one activation
    let src = "{i: i~ 0 >> loop.?} loop$ 200000 i# loop.. i~ >";
    let mut rt = Rt::new("", BTreeMap::new(), 1);
    let o = rt.run_program(src, &[]);
    assert_eq!((o.out.as_str(), o.err), ("\n 0", None));
    assert!(rt.max_depth <= 2, "depth {}", rt.max_depth);
    // a call that isn't last nests, and comes back
    assert_eq!(out("{in >} f$ f.. out >"), "\n in\n out");
}

#[test]
fn definitions_have_generations() {
    assert_eq!(out("std/core.s. {2 mul} dbl >< 5 dbl >"), "\n 10");
    // made before the library loaded, its body doesn't see `mul`
    assert_eq!(out("{2 mul} dbl >< std/core.s. 5 dbl >"), "\n 5 2 mul");
    // a definition's body sees only older ones: `x` inside its own body is the old x
    assert_eq!(out("{old} x >< {x new} 'x' >< x >"), "\n old new");
    // a script sees the definitions there were at its `$`
    assert_eq!(out("{greet >} s$ {hello} greet >< s.."), "\n greet");
}

#[test]
fn lists_tables_and_locals() {
    assert_eq!(out("x xs l y xs l xs out l/ out~ > xs #l =~ >"), "\n x y\n 2");
    assert_eq!(out("{a {1 >} b {2 >}} b ?)"), "\n 2");
    assert_eq!(out("{a {1 >}} z {3 >} ?)"), "\n 3");
    assert_eq!(out("{5 :x# :x~ >} f$ f.. x~ >"), "\n 5\n 404");
    assert_eq!(out("{a,b,c} parts , /l parts #l =~ >"), "\n 3");
}

#[test]
fn the_library() {
    assert_eq!(out("std/line.s. 1 2 3 {2 mul} map >"), "\n 2 4 6");
    assert_eq!(out("std/line.s. 1 2 3 4 sum >"), "\n 10");
    assert_eq!(out("std/line.s. 1 2 3 4 {add} fold >"), "\n 10");
    assert_eq!(out("std/line.s. 5 1 4 sort >"), "\n 1 4 5");
    assert_eq!(out("std/text.s. hello upper > {a b c} words count >"), "\n HELLO\n 3");
    assert_eq!(out("std/text.s. 1 2 {% and %} format >"), "\n 1 and 2");
    assert_eq!(out("std/gem.s. abc ord > 999 droot >"), "\n 6\n 9");
    assert_eq!(out("std/logic.s. 1 1 0 fadd >"), "\n 0 1");
    assert_eq!(out("std/elec.s. 12 1000 1000 divider >"), "\n 6");
    assert_eq!(out("std/text.s. p name ann set p name get >"), "\n ann");
    // a word used before the module loads isn't the library's
    assert_eq!(out("drop > std/core.s. a b drop >"), "\n drop\n a");
}

#[test]
fn errors_and_exits() {
    let o = run("+", "", BTreeMap::new(), &[], 1000, 7);
    assert_eq!(o.err.as_deref(), Some("+ needs 2 values"));
    assert_eq!(o.code, 1);
    let o = run("hi > 3 <-- never >", "", BTreeMap::new(), &[], 1000, 7);
    assert_eq!((o.out.as_str(), o.code, o.err), ("\n hi", 3, None));
    let o = run("{loop.} loop$ loop..", "", BTreeMap::new(), &[], 1000, 7);
    assert!(o.err.unwrap().contains("steps"));
    assert!(run("{a", "", BTreeMap::new(), &[], 10, 7).err.is_some());
}

#[test]
fn files_input_and_args() {
    let mut files = BTreeMap::new();
    files.insert("lib.s".to_string(), "{from a file} >".to_string());
    let o = run(
        "lib.s. a.txt hi w a.txt r a.txt~ > _ <_ _~ > args 0 x [l] x~ >",
        "typed\n",
        files,
        &["first".into()],
        1000,
        7,
    );
    assert_eq!(o.out, "\n from a file\n hi\n typed\n first");
    let o = run("{echo hi} < r~ >_", "", BTreeMap::new(), &[], 100, 7);
    assert_eq!(o.out, "hi\n");
}

#[test]
fn inline_results_and_stepping() {
    let src = "1 a#\n{one} >\n{x} sh$ sh. //\na~ 2 + =~ >";
    let (o, w) = super::run_traced(src, "", BTreeMap::new(), &[], 1000, 7);
    assert_eq!(o.out, "\n one\n 3");
    // the output by the program's token that wrote it: `>` on line 2, the call on line 3
    // wrote nothing (the script didn't print), `>` on line 4
    let lines: Vec<(usize, &str)> = w
        .iter()
        .map(|(r, t)| (src[..r.start].matches('\n').count(), t.as_str()))
        .collect();
    assert_eq!(lines, [(1, "\n one"), (3, "\n 3")]);
    // stepping: before step 3 the program is about to run `{one}` with nothing stacked
    let s = super::debug(src, "", BTreeMap::new(), &[], 2, 7);
    assert!(!s.done && s.main);
    assert_eq!(&src[s.at.clone().unwrap()], "{one}");
    assert_eq!(s.numbers["a"], "1");
    assert!(s.stack.is_empty());
    let s = super::debug(src, "", BTreeMap::new(), &[], 3, 7);
    assert_eq!(s.stack, ["one"]);
    // inside the script's body: not the program's own unit
    let s = super::debug(src, "", BTreeMap::new(), &[], 7, 7);
    assert!(!s.main && s.depth == 2, "{s:?}");
    assert_eq!(s.unit, "x");
    let s = super::debug(src, "", BTreeMap::new(), &[], 1000, 7);
    assert!(s.done && s.err.is_none());
}
