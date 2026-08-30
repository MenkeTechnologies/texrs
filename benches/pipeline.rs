//! Where the time goes in a run: the mouth, the frontend, and the VM.
//!
//! Four measurements over the same document, each isolating one more stage:
//!
//!   * `mouth`     — tokenising alone, no expansion.
//!   * `frontend`  — mouth + expander + lowering + bytecode emission.
//!   * `run`       — the whole pipeline, executing on fusevm.
//!   * `execute`   — the VM alone, on a chunk compiled once outside the loop.
//!
//! The pairs are what make the numbers readable: `frontend` minus `mouth` is
//! what expansion costs, and `run` minus `frontend` is what the VM costs. A
//! single end-to-end number cannot separate a slow expander from a slow VM.
//!
//! ```sh
//! cargo bench
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use texrs::catcode::CatTable;
use texrs::lexer::Lexer;

/// A document with the shapes a macro-heavy file spends its time in: a macro
/// with a delimited parameter called repeatedly, register arithmetic, and a
/// conditional inside a message body.
fn document(repeats: usize) -> String {
    let mut src = String::from(
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
         \\def\\greet#1{HELLO-#1}\n\
         \\def\\pair#1,#2.{[#1|#2]}\n\
         \\count1=0\n",
    );
    for i in 0..repeats {
        src.push_str("\\advance\\count1 by 3\n");
        src.push_str("\\multiply\\count1 by 2\n");
        src.push_str(&format!("\\message{{\\greet{{W{i}}} \\pair 1,2.}}\n"));
        src.push_str("\\message{\\ifnum\\count1>100 BIG\\else SMALL\\fi}\n");
    }
    src.push_str("\\end\n");
    src
}

fn pipeline(c: &mut Criterion) {
    // Big enough that per-iteration setup does not dominate, small enough that
    // the whole group runs in seconds.
    let src = document(200);
    let chunk = texrs::compile(&src).expect("compiles");

    let mut g = c.benchmark_group("pipeline");
    g.bench_function("mouth", |b| {
        b.iter(|| {
            let cats = CatTable::new();
            let mut lx = Lexer::new(black_box(&src));
            let mut n = 0usize;
            while lx.next_token(&cats).is_some() {
                n += 1;
            }
            black_box(n)
        })
    });
    g.bench_function("frontend", |b| {
        b.iter(|| black_box(texrs::compile(black_box(&src)).expect("compiles")))
    });
    g.bench_function("run", |b| {
        b.iter(|| black_box(texrs::run_messages(black_box(&src)).expect("runs")))
    });
    g.bench_function("execute", |b| {
        b.iter(|| black_box(texrs::runtime::run(black_box(chunk.clone())).expect("runs")))
    });
    g.finish();
}

/// How the cost scales with document size. A frontend whose expander is
/// quadratic in the number of macro calls looks fine on a small file and stops
/// being usable on a real one, and only a size sweep shows it.
fn scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("scaling");
    for repeats in [50usize, 200, 800] {
        let src = document(repeats);
        g.bench_function(format!("run/{repeats}"), |b| {
            b.iter(|| black_box(texrs::run_messages(black_box(&src)).expect("runs")))
        });
    }
    g.finish();
}

criterion_group!(benches, pipeline, scaling);
criterion_main!(benches);
