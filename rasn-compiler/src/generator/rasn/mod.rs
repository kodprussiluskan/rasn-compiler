use std::{
    collections::BTreeMap,
    env,
    error::Error,
    io::{self, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
};

use crate::{error::CompilerError, intermediate::*};
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};

#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::*;

use super::{
    error::{GeneratorError, GeneratorErrorType},
    Backend, GeneratedModule,
};

mod builder;
mod copy;
mod real;
mod storage;
mod template;
mod utils;
mod values;

use copy::{type_declarations, TypeDeclarations};

#[derive(Debug)]
/// A compiler backend that generates bindings to be used with
/// the `rasn` framework for rust.
pub struct Rasn {
    config: Config,
    tagging_environment: TaggingEnvironment,
    extensibility_environment: ExtensibilityEnvironment,
    /// A combination of the builtin required derives (`Rasn::REQUIRED_DERIVES`) and those supplied
    /// by the user in `Config::type_annotations`.
    required_derives: Vec<String>,
    copy_types: copy::CopyTypes,
    copy_module: Option<ModuleHeader>,
    fixed_size_arrays: bool,
    upper_camel_case_variants: bool,
    hashable_reals: bool,
    collection_layout: Option<storage::CollectionLayout>,
    collection_owner: std::cell::RefCell<Option<String>>,
    value_types: BTreeMap<(String, String), ToplevelTypeDefinition>,
}

#[cfg_attr(target_family = "wasm", wasm_bindgen(getter_with_clone))]
#[derive(Debug)]
/// A configuration for the [Rasn] backend
pub struct Config {
    /// ASN.1 Open Types are represented as the `rasn::types::Any` type,
    /// which holds a binary `content`. If `opaque_open_types` is `false`,
    /// the compiler will generate additional de-/encode methods for
    /// all rust types that hold an open type.
    /// For example, bindings for a `SEQUENCE` with a field of Open Type
    /// value will include a method for explicitly decoding the Open Type field.
    /// _Non-opaque open types are still experimental. If you have trouble_
    /// _generating correct bindings, switch back to opaque open types._
    pub opaque_open_types: bool,
    /// The compiler will try to match module import dependencies of the ASN.1
    /// module as close as possible, importing only those types from other modules
    /// that are imported in the ASN.1 module. If the `default_wildcard_imports`
    /// is set to `true` , the compiler will import the entire module using
    /// the wildcard `*` for each module that the input ASN.1 module imports from.
    pub default_wildcard_imports: bool,
    /// To make working with the generated types a bit more ergonomic, the compiler
    /// can generate `From` impls for the wrapper inner types in a `CHOICE`, as long
    /// as the generated impls are not ambiguous.
    /// This is disabled by default to generate less code, but can be enabled with
    /// `generate_from_impls` set to `true`.
    pub generate_from_impls: bool,
    /// Stringified paths to items that will be imported into all generated modules with a
    /// [use declaration](https://doc.rust-lang.org/reference/items/use-declarations.html).
    /// For example `vec![String::from("my::module::*"), String::from("path::to::my::Struct")]`.
    pub custom_imports: Vec<String>,
    /// Annotations to be added to all generated rust types of the bindings. Each vector element
    /// will generate a new line of annotations. Note that the compiler will automatically add all pound-derives
    /// needed by `rasn` __except__ `Eq` and `Hash`, which are needed only when working with `SET`s.
    ///
    /// The backend infers additional `Copy` derives when custom imports and non-derive
    /// annotations are absent, since these may affect field types or generated implementations.
    ///
    /// Default: `vec![String::from("#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]")]`
    pub type_annotations: Vec<String>,
    /// Create bindings for a `no_std` environment
    pub no_std_compliant_bindings: bool,
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen]
impl Config {
    #[wasm_bindgen(constructor)]
    pub fn new(
        opaque_open_types: bool,
        default_wildcard_imports: bool,
        no_std_compliant_bindings: bool,
        generate_from_impls: Option<bool>,
        custom_imports: Option<Box<[String]>>,
        type_annotations: Option<Box<[String]>>,
    ) -> Self {
        Self {
            opaque_open_types,
            default_wildcard_imports,
            no_std_compliant_bindings,
            generate_from_impls: generate_from_impls.unwrap_or(false),
            custom_imports: custom_imports.map_or(Vec::new(), |c| c.into_vec()),
            type_annotations: type_annotations
                .map_or(Config::default().type_annotations, |c| c.into_vec()),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            opaque_open_types: true,
            default_wildcard_imports: false,
            generate_from_impls: false,
            no_std_compliant_bindings: false,
            custom_imports: Vec::default(),
            type_annotations: vec![String::from(
                "#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]",
            )],
        }
    }
}

impl Backend for Rasn {
    type Config = Config;

    const FILE_EXTENSION: &'static str = ".rs";

    fn new(
        mut config: Self::Config,
        tagging_environment: TaggingEnvironment,
        extensibility_environment: ExtensibilityEnvironment,
    ) -> Self {
        // Remove derive's from config.type_annotations, so that they can be combined with required
        // derives, and handled more optimally in later steps.
        let mut required_derives: Vec<_> = Self::REQUIRED_DERIVES
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut non_derive_annotations = Vec::new();
        for cfg_annotation in config.type_annotations {
            if let Ok((_, derives)) = parse_rust_derive_annotation(&cfg_annotation) {
                for derive in derives {
                    if !required_derives.iter().any(|d| d == derive) {
                        required_derives.push(derive.to_owned());
                    }
                }
            } else {
                non_derive_annotations.push(cfg_annotation);
            }
        }
        config.type_annotations = non_derive_annotations;

        Self {
            config,
            extensibility_environment,
            tagging_environment,
            required_derives,
            copy_types: copy::CopyTypes::default(),
            copy_module: None,
            fixed_size_arrays: false,
            upper_camel_case_variants: false,
            hashable_reals: false,
            collection_layout: None,
            collection_owner: std::cell::RefCell::default(),
            value_types: BTreeMap::new(),
        }
    }

    fn from_config(config: Self::Config) -> Self {
        Self::new(
            config,
            TaggingEnvironment::default(),
            ExtensibilityEnvironment::default(),
        )
    }

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn generate_module(
        &mut self,
        tlds: Vec<ToplevelDefinition>,
    ) -> Result<GeneratedModule, GeneratorError> {
        self.generate_modules(vec![tlds])
            .map(|mut modules| modules.pop().unwrap())
    }

    fn generate_modules(
        &mut self,
        modules: Vec<Vec<ToplevelDefinition>>,
    ) -> Result<Vec<GeneratedModule>, GeneratorError> {
        self.value_types = modules
            .iter()
            .flatten()
            .filter_map(|definition| {
                let ToplevelDefinition::Type(declaration) = definition else {
                    return None;
                };
                let header = declaration.module_header.as_ref()?.borrow();
                Some((
                    (header.name.clone(), declaration.name.clone()),
                    declaration.clone(),
                ))
            })
            .collect();
        let declarations = type_declarations(&modules);
        self.collection_layout = Some(self.collect_collection_layout(&declarations)?);
        self.copy_types = self.collect_copy_types(&modules, &declarations);
        let real_constants: Vec<_> = modules
            .iter()
            .map(|module| {
                module
                    .iter()
                    .filter_map(|definition| {
                        let ToplevelDefinition::Value(value) = definition else {
                            return None;
                        };
                        self.generate_real_constant(value, &declarations)
                            .map(|constant| (value.name.clone(), constant))
                    })
                    .collect()
            })
            .collect();
        let generated = modules
            .into_iter()
            .zip(real_constants)
            .map(|(module, constants)| self.render_module(module, constants))
            .collect();
        self.copy_types = copy::CopyTypes::default();
        self.collection_layout = None;
        self.value_types.clear();
        generated
    }

    fn format_bindings(bindings: &str) -> Result<String, CompilerError> {
        Self::internal_fmt(bindings).map_err(|e| {
            GeneratorError {
                top_level_declaration: None,
                details: e.to_string(),
                kind: GeneratorErrorType::FormattingError,
            }
            .into()
        })
    }

    fn generate(&self, tld: ToplevelDefinition) -> Result<String, GeneratorError> {
        if let ToplevelDefinition::Value(value) = &tld {
            if let Some(constant) = self.generate_real_constant(value, &TypeDeclarations::new()) {
                return constant.map(|tokens| tokens.to_string());
            }
        }
        self.generate_tld(tld).map(|ts| ts.to_string())
    }
}

impl Rasn {
    /// Enables ordinary Rust arrays for provably fixed-size SEQUENCE OF collections.
    ///
    /// The default is false. Named ASN.1 collections retain their delegating wrapper.
    /// Array storage changes public field types and constructors, and decoding still
    /// allocates a temporary vector. Extensible sizes, recursive layout edges and
    /// named subtype references keep their existing collection representation.
    /// Standalone `Backend::generate` uses vectors because it lacks batch layout analysis.
    ///
    /// Counts must fit u32, making generated count literals portable across supported
    /// 32-bit and 64-bit targets. Large inline arrays can exceed target layout limits
    /// or the available stack for their element type.
    ///
    /// ```
    /// use rasn_compiler::prelude::{Compiler, RasnBackend};
    ///
    /// let result = Compiler::<RasnBackend, _>::new()
    ///     .with_backend(RasnBackend::default().with_fixed_size_arrays(true))
    ///     .add_asn_literal("Example DEFINITIONS ::= BEGIN Flags ::= SEQUENCE SIZE (3) OF BOOLEAN END")
    ///     .compile_to_string()
    ///     .unwrap();
    /// assert!(result.warnings.is_empty());
    /// // Flags wraps [AnonymousFlags; 3]. The default backend uses SequenceOf.
    /// ```
    pub fn with_fixed_size_arrays(mut self, enabled: bool) -> Self {
        self.fixed_size_arrays = enabled;
        self
    }

    /// Name ENUMERATED items and CHOICE alternatives in Rust's UpperCamelCase (`installation-read`
    /// becomes `InstallationRead`) instead of the default, which only replaces `-` with `_`.
    /// Encodings are unchanged: a renamed variant keeps its ASN.1 name in `#[rasn(identifier)]`.
    ///
    /// ```
    /// use rasn_compiler::prelude::{Compiler, RasnBackend};
    ///
    /// let result = Compiler::<RasnBackend, _>::new()
    ///     .with_backend(RasnBackend::default().with_upper_camel_case_variants(true))
    ///     .add_asn_literal("Example DEFINITIONS ::= BEGIN Kind ::= ENUMERATED { read-only, write } END")
    ///     .compile_to_string()
    ///     .unwrap();
    /// assert!(result.generated.contains("ReadOnly"));
    /// ```
    pub fn with_upper_camel_case_variants(mut self, enabled: bool) -> Self {
        self.upper_camel_case_variants = enabled;
        self
    }

    /// Generate every top-level REAL type as a newtype over its `f32` or `f64` with `Eq` and
    /// `Hash`, instead of a plain type alias. Values compare as numbers, except that NaN equals
    /// NaN, and hash consistently with that (`-0.0` like `0.0`, every NaN alike). Types holding
    /// these REALs then derive `Eq` and `Hash` too and can be elements of a SET OF. A REAL
    /// written inline in another type stays a plain float. Encodings are unchanged: the newtype
    /// is `#[rasn(delegate)]`, and `#[repr(transparent)]` over its float.
    ///
    /// ```
    /// use rasn_compiler::prelude::{Compiler, RasnBackend};
    ///
    /// let result = Compiler::<RasnBackend, _>::new()
    ///     .with_backend(RasnBackend::default().with_hashable_reals(true))
    ///     .add_asn_literal(
    ///         "Example DEFINITIONS ::= BEGIN
    ///          Float32 ::= REAL (WITH COMPONENTS { mantissa (-16777215..16777215), base (2), exponent (-149..104) })
    ///          END",
    ///     )
    ///     .compile_to_string()
    ///     .unwrap();
    /// assert!(result.generated.contains("pub struct Float32(pub f32)"));
    /// ```
    pub fn with_hashable_reals(mut self, enabled: bool) -> Self {
        self.hashable_reals = enabled;
        self
    }

    const REQUIRED_DERIVES: [&'static str; 6] =
        ["AsnType", "Debug", "Clone", "Decode", "Encode", "PartialEq"];

    fn uses_wildcard_import(&self, import: &Import) -> bool {
        self.config.default_wildcard_imports
            || import.types.iter().any(|usage| {
                usage.chars().all(|c| c.is_uppercase() || c == '-')
            })
    }

    fn render_module(
        &mut self,
        tlds: Vec<ToplevelDefinition>,
        mut real_constants: BTreeMap<String, Result<TokenStream, GeneratorError>>,
    ) -> Result<GeneratedModule, GeneratorError> {
        if let Some(module_ref) = tlds.first().and_then(|tld| tld.get_module_header()) {
            let module = module_ref.borrow();
            self.tagging_environment = module.tagging_environment;
            self.extensibility_environment = module.extensibility_environment;
            let name = self.to_rust_snake_case(&module.name);
            let custom_imports = self
                .config
                .custom_imports
                .iter()
                .map(|i| TokenStream::from_str(i.as_str()).map(|i| quote!(use #i;)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| GeneratorError {
                    details: e.to_string(),
                    ..Default::default()
                })?;
            self.copy_module = Some(module.clone());
            let imports = module.imports.iter().map(|import| {
                let import_module =
                    self.to_rust_snake_case(&import.global_module_reference.module_reference);
                let used_imports = if self.uses_wildcard_import(import) {
                    vec![quote!(*)]
                } else {
                    import
                        .types
                        .iter()
                        .filter_map(|usage| {
                            if module.has_conflicting_import(usage)
                                || tlds.iter().any(|definition| definition.name() == usage)
                            {
                                return None;
                            }
                            if usage.starts_with(|c: char| c.is_lowercase()) {
                                Some(self.to_rust_const_case(usage).to_token_stream())
                            } else if usage.starts_with(|c: char| c.is_uppercase()) {
                                Some(self.to_rust_title_case(usage).to_token_stream())
                            } else {
                                None
                            }
                        })
                        .collect()
                };
                quote!(use super:: #import_module::{ #(#used_imports),* };)
            }).collect::<Vec<_>>();
            let (pdus, warnings): (Vec<TokenStream>, Vec<CompilerError>) =
                tlds.into_iter().fold((vec![], vec![]), |mut acc, tld| {
                    self.tagging_environment = tld
                        .get_module_header()
                        .map_or(module.tagging_environment, |header| {
                            header.borrow().tagging_environment
                        });
                    let generated = real_constants
                        .remove(tld.name())
                        .unwrap_or_else(|| self.generate_tld(tld));
                    match generated {
                        Ok(s) => {
                            acc.0.push(s);
                            acc
                        }
                        Err(e) => {
                            acc.1.push(e.into());
                            acc
                        }
                    }
                });
            let lazy_const_import = if self.config.no_std_compliant_bindings {
                quote!(lazy_static::lazy_static)
            } else {
                quote!(std::sync::LazyLock)
            };
            let generated = quote! {
                #[allow(non_camel_case_types, non_snake_case, non_upper_case_globals, unused,
                        clippy::too_many_arguments,)]
                pub mod #name {
                    extern crate alloc;

                    use core::borrow::Borrow;
                    use #lazy_const_import;
                    use rasn::prelude::*;
                    #(#custom_imports)*
                    #(#imports)*

                    #(#pdus)*
                }
            }
            .to_string();
            self.copy_module = None;
            Ok(GeneratedModule {
                generated: Some(generated),
                warnings,
            })
        } else {
            Ok(GeneratedModule::empty())
        }
    }

    fn get_rustfmt_path() -> Result<PathBuf, Box<dyn Error>> {
        // Try ~/.cargo/bin/rustfmt style paths first
        if let Ok(path) = env::var("CARGO_HOME").map(PathBuf::from).map(|mut path| {
            path.push("bin/rustfmt");
            path
        }) {
            if path.exists() {
                return Ok(path);
            }
        }

        // Alternatively, maybe rustfmt and cargo are in the same directory
        if let Ok(path) = env::var("CARGO").map(PathBuf::from).map(|mut path| {
            path.set_file_name("rustfmt");
            path
        }) {
            if path.exists() {
                return Ok(path);
            }
        }

        Err("No rustfmt found.".into())
    }

    fn internal_fmt(bindings: &str) -> Result<String, Box<dyn Error>> {
        let rustfmt = Self::get_rustfmt_path()?;
        let mut cmd = Command::new(rustfmt);

        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());

        let mut child = cmd.spawn()?;
        let mut child_stdin = child.stdin.take().unwrap();
        let mut child_stdout = child.stdout.take().unwrap();

        // Write to stdin in a new thread, so that we can read from stdout on this
        // thread. This keeps the child from blocking on writing to its stdout which
        // might block us from writing to its stdin.
        let bindings = bindings.to_owned();
        let stdin_handle = ::std::thread::spawn(move || {
            let _ = child_stdin.write_all(bindings.as_bytes());
            bindings
        });

        let mut output = vec![];
        io::copy(&mut child_stdout, &mut output)?;

        let status = child.wait()?;
        let bindings = stdin_handle.join().expect(
            "The thread writing to rustfmt's stdin doesn't do \
             anything that could panic",
        );

        match String::from_utf8(output) {
            Ok(bindings) => match status.code() {
                Some(0) => Ok(bindings),
                Some(2) => Err(Box::new(io::Error::other("Rustfmt parsing errors."))),
                Some(3) => Ok(bindings),
                _ => Err(Box::new(io::Error::other("Internal rustfmt error"))),
            },
            _ => Ok(bindings),
        }
    }
}

fn parse_rust_derive_annotation(input: &str) -> nom::IResult<&str, Vec<&str>> {
    use nom::{
        bytes::complete::tag,
        character::complete::{alphanumeric1, char, multispace0},
        multi::{many0, separated_list1},
        sequence::delimited,
        Parser as _,
    };

    delimited(
        (
            multispace0,
            char('#'),
            multispace0,
            char('['),
            multispace0,
            tag("derive"),
            multispace0,
            char('('),
            multispace0,
        ),
        separated_list1(many0((multispace0, char(','), multispace0)), alphanumeric1),
        (multispace0, char(')'), multispace0, char(']')),
    )
    .parse(input)
}

impl Default for Rasn {
    fn default() -> Self {
        Self::new(
            Config::default(),
            TaggingEnvironment::default(),
            ExtensibilityEnvironment::default(),
        )
    }
}
