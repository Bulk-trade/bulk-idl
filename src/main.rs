use clap::Parser;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use syn::{File, Item};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// Path to the program's Cargo.toml file
    manifest_path: PathBuf,

    /// Output path for the generated IDL file
    #[clap(short, long, default_value = "idl.json")]
    output: PathBuf,
}

#[derive(Serialize)]
struct Idl {
    version: String,
    name: String,
    instructions: Vec<IdlInstruction>,
    accounts: Vec<IdlAccount>,
    types: Vec<IdlType>,
}

#[derive(Serialize)]
struct IdlInstruction {
    name: String,
    args: Vec<IdlArgument>,
    accounts: Vec<IdlAccountMeta>,
}

#[derive(Serialize)]
struct IdlArgument {
    name: String,
    type_name: String,
}

#[derive(Serialize)]
struct IdlAccountMeta {
    name: String,
    is_mut: bool,
    is_signer: bool,
}

#[derive(Serialize)]
struct IdlAccount {
    name: String,
    type_name: String,
    fields: Vec<IdlField>,
}

#[derive(Serialize)]
struct IdlField {
    name: String,
    type_name: String,
}

#[derive(Serialize)]
struct IdlType {
    name: String,
    type_def: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // Expand the entire crate
    let expanded_code = expand_crate(&args.manifest_path)?;

    // Parse the expanded code
    let ast = parse_source_file(&expanded_code)?;

    // Collect metadata
    let instructions = collect_instructions(&ast);
    let accounts = collect_accounts(&ast);
    let types = collect_types(&ast);

    // Extract program name from Cargo.toml
    let program_name = extract_program_name(&args.manifest_path)?;

    // Create the IDL
    let idl = Idl {
        version: "0.1.0".to_string(),
        name: program_name,
        instructions,
        accounts,
        types,
    };

    // Serialize to JSON
    let idl_json = serde_json::to_string_pretty(&idl)?;
    fs::write(&args.output, idl_json)?;

    println!("IDL generated at {}", args.output.display());

    Ok(())
}

fn expand_crate(manifest_path: &PathBuf) -> io::Result<String> {
    // Use `cargo expand` to expand the entire crate
    let output = Command::new("cargo")
        .args(&[
            "expand",
            "--manifest-path",
            manifest_path.to_str().unwrap(),
            "--lib",
        ])
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        Err(io::Error::new(io::ErrorKind::Other, "Failed to expand macros"))
    }
}

fn parse_source_file(file_content: &str) -> syn::Result<File> {
    syn::parse_file(file_content)
}

fn collect_instructions(ast: &File) -> Vec<IdlInstruction> {
    let mut instructions = Vec::new();

    for item in &ast.items {
        if let Item::Fn(item_fn) = item {
            if is_instruction_fn(item_fn) {
                instructions.push(parse_instruction_fn(item_fn));
            }
        }
    }

    instructions
}

fn is_instruction_fn(item_fn: &syn::ItemFn) -> bool {
    matches!(item_fn.vis, syn::Visibility::Public(_))
}

fn parse_instruction_fn(item_fn: &syn::ItemFn) -> IdlInstruction {
    let name = item_fn.sig.ident.to_string();
    let args = item_fn
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            syn::FnArg::Typed(pat_type) => {
                let arg_name = match &*pat_type.pat {
                    syn::Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
                    _ => "_".to_string(),
                };
                let type_name = type_to_string(&*pat_type.ty);
                Some(IdlArgument { name: arg_name, type_name })
            }
            _ => None,
        })
        .collect();

    let accounts = Vec::new(); // Collect accounts if possible

    IdlInstruction {
        name,
        args,
        accounts,
    }
}

fn type_to_string(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string()
}

fn collect_accounts(ast: &File) -> Vec<IdlAccount> {
    let mut accounts = Vec::new();

    for item in &ast.items {
        if let Item::Struct(item_struct) = item {
            if is_account_struct(item_struct) {
                accounts.push(parse_account_struct(item_struct));
            }
        }
    }

    accounts
}

fn is_account_struct(item_struct: &syn::ItemStruct) -> bool {
    // Check for attributes that indicate an account struct
    item_struct.attrs.iter().any(|attr| {
        attr.path().is_ident("account") || attr.path().is_ident("derive")
    })
}

fn parse_account_struct(item_struct: &syn::ItemStruct) -> IdlAccount {
    let name = item_struct.ident.to_string();
    let fields = item_struct
        .fields
        .iter()
        .filter_map(|field| {
            let field_name = field.ident.as_ref()?.to_string();
            let type_name = type_to_string(&field.ty);
            Some(IdlField { name: field_name, type_name })
        })
        .collect();

    IdlAccount {
        name: name.clone(),
        type_name: name,
        fields,
    }
}

fn collect_types(ast: &File) -> Vec<IdlType> {
    let mut types = Vec::new();

    for item in &ast.items {
        match item {
            Item::Struct(item_struct) => {
                if !is_account_struct(item_struct) {
                    types.push(parse_type_struct(item_struct));
                }
            }
            Item::Enum(item_enum) => {
                types.push(parse_type_enum(item_enum));
            }
            _ => {}
        }
    }

    types
}

fn parse_type_struct(item_struct: &syn::ItemStruct) -> IdlType {
    let name = item_struct.ident.to_string();
    let type_def = quote::quote!(#item_struct).to_string();

    IdlType { name, type_def }
}

fn parse_type_enum(item_enum: &syn::ItemEnum) -> IdlType {
    let name = item_enum.ident.to_string();
    let type_def = quote::quote!(#item_enum).to_string();

    IdlType { name, type_def }
}

fn extract_program_name(manifest_path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    let manifest_content = fs::read_to_string(manifest_path)?;
    let manifest: toml::Value = toml::from_str(&manifest_content)?;

    let package = manifest
        .get("package")
        .and_then(|p| p.as_table())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Invalid Cargo.toml: missing [package]"))?;

    let name = package
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Invalid Cargo.toml: missing package name"))?;

    Ok(name.to_string())
}
