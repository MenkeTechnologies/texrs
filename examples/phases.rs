use std::time::Instant;
use texrs::catcode::CatTable;
use texrs::lexer::Lexer;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let src = std::fs::read_to_string(&path).expect("read");
    let reps = 7;
    let mut lex = f64::MAX;
    let mut front = f64::MAX;
    let mut codegen = f64::MAX;
    let mut total = f64::MAX;
    for _ in 0..reps {
        let cats = CatTable::new();
        let t = Instant::now();
        let mut lx = Lexer::new(&src);
        let mut n = 0usize;
        while lx.next_token(&cats).is_some() {
            n += 1;
        }
        lex = lex.min(t.elapsed().as_secs_f64());
        std::hint::black_box(n);

        let t = Instant::now();
        let cmds = texrs::lower::Lowerer::new().lower(&src).expect("lower");
        front = front.min(t.elapsed().as_secs_f64());

        let t = Instant::now();
        let chunk = texrs::compiler::Compiler::new()
            .compile(&cmds)
            .expect("compile");
        codegen = codegen.min(t.elapsed().as_secs_f64());
        std::hint::black_box(chunk.ops.len());

        let t = Instant::now();
        let _ = texrs::run_messages(&src).expect("run");
        total = total.min(t.elapsed().as_secs_f64());
    }
    println!("  mouth alone (lex)      {lex:.4}s");
    println!(
        "  lex+expand+lower       {front:.4}s   (expand+lower = {:.4}s)",
        front - lex
    );
    println!("  codegen                {codegen:.4}s");
    println!(
        "  whole run (incl VM)    {total:.4}s   (VM+rest = {:.4}s)",
        total - front - codegen
    );
}
