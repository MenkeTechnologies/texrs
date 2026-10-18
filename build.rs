//! The C libraries LuaTeX links into its Lua, compiled against mlua's Lua.
//!
//! LuaTeX's `luainterpreter()` (luatexdir/lua/luastuff.c) opens `lpeg`,
//! `bit32`, `unicode`, `md5`, `sha2` and `lfs` beside Lua's own libraries, and
//! every serious Lua package for LuaTeX assumes them: lualibs and the
//! luaotfload font loader are written in LPeg from end to end. Three of them
//! are C libraries whose behaviour IS their source -- an LPeg pattern, a
//! slnunicode case mapping and an `lfs.attributes` table are whatever that C
//! computes -- so they are compiled here from the copies LuaTeX itself ships
//! (`vendor/luatex/`, each beside its licence), rather than re-implemented.
//! `zlib` and `gzip` (luazlib) are bindings to zlib, compiled against the zlib
//! sources `libz-sys` builds (`DEP_Z_INCLUDE`). `md5` and `sha2` are digests
//! whose answers are fixed by their RFCs, and are in Rust (`src/lua/libs.rs`).
//!
//! The headers are mlua's vendored Lua 5.3's, handed over by Cargo as
//! `DEP_LUA_INCLUDE` because this crate depends on `mlua-sys` directly, so the
//! libraries and the interpreter agree on every structure and every macro.
//! `LUA_COMPAT_5_2` is what TeX Live's lua53 is configured with
//! (libs/lua53/configure.ac: `LUA53_DEFINES="$LUA53_DEFINES -DLUA_COMPAT_5_2"`),
//! and under it `luaconf.h` turns on `LUA_COMPAT_BITLIB`, which is the guard
//! `lbitlib.c` is written behind. `LUA_COMPAT_MODULE` is on in TeX Live's Lua
//! as well -- a luatex run has `module` and `package.seeall`, which exist only
//! under it -- and slnunicode needs its `luaL_openlib`.

fn main() {
    let include = std::env::var("DEP_LUA_INCLUDE")
        .expect("mlua-sys (vendored) hands its Lua include directory to this build script");
    let zlib = std::env::var("DEP_Z_INCLUDE")
        .expect("libz-sys (static) hands its zlib include directory to this build script");
    let vendor = std::path::Path::new("vendor/luatex");
    let sources = [
        "lpeg/lpeg.c",
        "slnunicode/slnunico.c",
        "luafilesystem/lfs.c",
        "lua53/lbitlib.c",
        "lua53/compat.c",
        "luazlib/lzlib.c",
        "luazlib/lgzip.c",
    ];
    let mut build = cc::Build::new();
    build
        .include(&include)
        .include(&zlib)
        .include(vendor.join("lpeg"))
        .include(vendor.join("lua53"))
        .define("LUA_COMPAT_5_2", None)
        // slnunicode's `luaopen_unicode` calls `luaL_openlib`, declared only
        // under this switch; `lua53/compat.c` supplies its body.
        .define("LUA_COMPAT_MODULE", None)
        // The sources are other projects' and are compiled as they ship;
        // their warnings are theirs to fix, not this build's to print.
        .warnings(false);
    for s in sources {
        let path = vendor.join(s);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }
    // `slnunico.c` `#include`s its data table, so a change there is a change
    // to the library.
    println!("cargo:rerun-if-changed=vendor/luatex/slnunicode/slnudata.c");
    println!("cargo:rerun-if-changed=vendor/luatex/lpeg/lpeg.h");
    println!("cargo:rerun-if-changed=build.rs");
    build.compile("luatex_c");
}
