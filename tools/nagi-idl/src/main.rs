use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const RUST_BINDINGS: &str = r#"// Generated from idl/application.nidl; do not edit by hand.
pub const APPLICATION_SERVICE_NAME: &str = "application";
pub const APPLICATION_SERVICE_VERSION: u16 = 1;
pub const APPLICATION_OPEN_OPERATION: u16 = 1;
pub const APPLICATION_SURFACE_OPERATION: u16 = 2;
"#;

const C_HEADER: &str = r#"/* Generated from idl/application.nidl; do not edit by hand. */
#ifndef NAGI_SDK_H
#define NAGI_SDK_H

#include <stdint.h>

typedef struct { uint64_t value; } nagi_app_id_t;
typedef struct { uint64_t value; } nagi_app_session_id_t;
typedef struct { uint64_t value; } nagi_surface_id_t;
typedef struct { uint64_t value; } nagi_node_id_t;

typedef enum {
    NAGI_PRESENTATION_COMPACT = 0,
    NAGI_PRESENTATION_MEDIUM = 1,
    NAGI_PRESENTATION_EXPANDED = 2
} nagi_presentation_class_t;

typedef struct {
    uint16_t logical_width;
    uint16_t logical_height;
    uint16_t dpi;
    uint8_t touch;
    uint8_t keyboard;
    uint8_t pointer;
    nagi_presentation_class_t class_id;
} nagi_presentation_context_t;

typedef struct {
    nagi_app_id_t app_id;
    nagi_app_session_id_t session_id;
} nagi_application_t;

nagi_application_t nagi_application_open(nagi_app_id_t app_id, nagi_app_session_id_t session_id);
int nagi_application_surface(const nagi_application_t *application,
                             nagi_presentation_context_t context,
                             nagi_surface_id_t *surface_id,
                             nagi_node_id_t *node_id);

#endif
"#;

const C_SOURCE: &str = r#"/* Generated from idl/application.nidl; do not edit by hand. */
#include "nagi_sdk.h"

nagi_application_t nagi_application_open(nagi_app_id_t app_id,
                                         nagi_app_session_id_t session_id) {
    nagi_application_t application = { app_id, session_id };
    return application;
}

int nagi_application_surface(const nagi_application_t *application,
                             nagi_presentation_context_t context,
                             nagi_surface_id_t *surface_id,
                             nagi_node_id_t *node_id) {
    if (application == 0 || surface_id == 0 || node_id == 0 ||
        context.logical_width == 0 || context.logical_height == 0) {
        return -1;
    }
    surface_id->value = application->session_id.value;
    node_id->value = 1;
    return 0;
}
"#;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("generate") => {
            let source = PathBuf::from(args.next().unwrap_or_else(|| usage("missing IDL path")));
            let output = PathBuf::from(
                args.next()
                    .unwrap_or_else(|| usage("missing output directory")),
            );
            if args.next().is_some() {
                usage("too many arguments");
            }
            generate(&source, &output);
            println!("PASS nagi-idl generate: {}", output.display());
        }
        _ => usage("usage: nagi-idl generate <idl> <output-directory>"),
    }
}

fn generate(source: &Path, output: &Path) {
    let text = fs::read_to_string(source).unwrap_or_else(|error| panic!("read IDL: {error}"));
    for required in [
        "service application@1",
        "application_open",
        "application_surface",
        "enum PresentationClass",
        "struct PresentationContext",
    ] {
        assert!(
            text.contains(required),
            "IDL missing required declaration: {required}"
        );
    }
    fs::create_dir_all(output.join("rust")).expect("create Rust output directory");
    fs::create_dir_all(output.join("c")).expect("create C output directory");
    fs::write(output.join("rust").join("application.rs"), RUST_BINDINGS)
        .expect("write Rust bindings");
    fs::write(output.join("c").join("nagi_sdk.h"), C_HEADER).expect("write C header");
    fs::write(output.join("c").join("nagi_sdk.c"), C_SOURCE).expect("write C source");
}

fn usage(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}
