// We need to forward routine registration from C to Rust
// to avoid the linker removing the static library.
//
// (This is the entrypoint rextendr generates for every extendr
// package; it is included verbatim in shape.)
void R_init_codetopo_extendr(void *dll);
void R_init_codetopo(void *dll) { R_init_codetopo_extendr(dll); }
