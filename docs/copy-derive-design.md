# Automatic `Copy` for generated rasn types

Generate an ordinary `#[derive(Copy)]` when rasn-compiler can establish that every field or variant payload in the emitted Rust type is `Copy`. Make the decision in the rasn backend, after linking and Rust storage selection. Existing user-authored rasn structs can use the ordinary Rust derive. This design needs no new rasn trait, derive macro, runtime code, or ASN.1 encoding rule. [Rust `Copy` requirements and public API guidance](https://doc.rust-lang.org/std/marker/trait.Copy.html)

## Base eligibility on emitted storage

| Emitted Rust storage | Eligibility |
| --- | --- |
| `()`, `bool`, fixed-width integers, `f32`, `f64` | Eligible |
| `FixedBitString<N>`, `FixedOctetString<N>`, `UtcTime`, `GeneralizedTime` | Eligible |
| `Option<T>` | Same eligibility as `T` |
| `Box<T>`, `SequenceOf<T>`, `SetOf<T>`, rasn `Integer`, variable-length strings, owned OIDs, `Any` | Ineligible |
| A generated named type | Use that specific declaration's eligibility |
| A reference whose Rust definition is unavailable or ambiguous | No automatic derive for types that depend on it |

Sources for storage and traits are [rasn primitive and time mappings](../../rasn/src/types.rs), [fixed bit storage](../../rasn/src/types/strings/bit.rs#L39), [bitvec's `Copy` implementation](https://docs.rs/bitvec/latest/bitvec/array/struct.BitArray.html#impl-Copy-for-BitArray%3CA,+O%3E), [fixed octet storage](../../rasn/src/types/strings/octet.rs#L145), [chrono's conditional `Copy` implementation](https://docs.rs/chrono/latest/chrono/struct.DateTime.html#impl-Copy-for-DateTime%3CTz%3E), [collection storage](../../rasn/src/types/constructed.rs#L39), [arbitrary-precision integer storage](../../rasn/src/types/integer.rs#L15), and [Rust's `Option`](https://doc.rust-lang.org/std/option/enum.Option.html#impl-Copy-for-Option%3CT%3E).

A `SEQUENCE` or `SET` qualifies when all its emitted fields qualify. The same rule applies to every payload of a `CHOICE`, and a delegating newtype has one field to check. An empty struct and a payload-free enum qualify. `DEFAULT` and `#[non_exhaustive]` impose no additional storage requirements. Assess extension additions and extension groups using their actual field types. A single boxed recursive field makes its container ineligible, even when that field is optional. [Sequence fields and optionality](../rasn-compiler/src/generator/rasn/utils.rs#L321), [recursive member lowering](../rasn-compiler/src/generator/rasn/utils.rs#L478), [struct template](../rasn-compiler/src/generator/rasn/template.rs#L272)

Two cases rule out a separate predicate on ASN.1 syntax. Named fixed-length octet strings use `FixedOctetString<N>`, while inline octet fields currently lower to `OctetString`. Named integers use `int.int_type()`, while inline integers select storage from the PER-visible range. Reuse those existing storage selectors when deciding whether to derive `Copy`. [Named wrappers](../rasn-compiler/src/generator/rasn/builder.rs#L158), [inline member storage](../rasn-compiler/src/generator/rasn/utils.rs#L556)

## Where to implement it

1. **Reuse the Rust storage decisions.** The backend uses the same integer selector for inline member emission and eligibility, and the same fixed-size checks for named string wrappers. `Option` forwards its inner type's eligibility, and recursive boxing and owned collections make a field ineligible. Time and fixed-string names are checked against declarations and imports so shadowing preserves the current Rust field type. The existing derive merger inserts `Copy`. Custom imports and non-derive type annotations suspend the new inference because they can change bindings or implementations. [Eligibility pass](../rasn-compiler/src/generator/rasn/copy.rs), [inline member lowering](../rasn-compiler/src/generator/rasn/utils.rs#L556)
2. **Resolve the dependencies for the whole compilation.** After validation, collect the Rust storage descriptions before rendering the modules. Memoize whether each declaration qualifies. For a named dependency, use its linked ASN.1 identity, including its module. Prefer an explicit module on the reference, then a local declaration or a uniquely resolved import as appropriate. Read the owning import records to recover their module names. Evaluate anonymous types in their enclosing module, using their structure directly for the eligibility decision. Forward references and module ordering should have no effect. Treat unresolved references and cycles as unproven. There is no need to follow a dependency inside `Box` or a collection. The existing validator indexes its declarations by bare name, so the derive pass must not use that map to guess across module collisions or promise to repair the linker's existing limitations. [Compilation flow](../rasn-compiler/src/lib.rs#L489), [validator's index](../rasn-compiler/src/validator/mod.rs#L35), [reference representation](../rasn-compiler/src/intermediate/mod.rs#L1272), [module import records](../rasn-compiler/src/intermediate/mod.rs#L353)
3. **Render ordinary derives.** A defaulted batch-generation method on `Backend` gives the rasn backend all linked modules at once. Its default keeps the existing per-module loop for TypeScript and other backends. The rasn backend clears the eligibility data after rendering the batch. Standalone `Backend::generate` gets the same leaf and inline rules, with unavailable named types treated as unknown. This covers fixed-size wrappers, time wrappers, self-contained composites, and references to linked types without changing field representations. [Backend interface](../rasn-compiler/src/generator/mod.rs#L17), [current rasn entry points](../rasn-compiler/src/generator/rasn/mod.rs#L166)

For example, a sequence with a boolean, an optional bounded integer and a field referring to a named fixed-size octet wrapper should derive `Copy` once that wrapper is generated with `Copy`. A sequence with a similarly constrained inline octet string still uses the current owned storage. Keep the default `Eq` and `Hash` policy separate. A direct `REAL` field can be `Copy` when generated with compatible derives and the rasn `f64` feature. Top-level `REAL` generation is a separate feature. [Inline type mapping](../rasn-compiler/src/generator/rasn/utils.rs#L556), [default derives](../rasn-compiler/src/generator/rasn/mod.rs#L102), [top-level REAL](../rasn-compiler/src/generator/rasn/builder.rs#L82), [rasn features](../../rasn/Cargo.toml#L32)

## Alternatives

**Conditional impls would let rustc check named types.** An ordinary concrete bound such as `where String: Copy` is rejected at the impl definition. A higher-ranked bound such as `where for<'a> String: Copy` defers checking and can make an impl usable only when its field bounds hold. Local compile probes with stable Rust 1.100 and nightly-2023-10-02 accepted a nested eligible struct and rejected use of the owning struct as `Copy`. Both toolchains also rejected a local trait implementation for the owning struct alongside a blanket implementation for `T: Copy`, and rejected adding `Drop` to the owning struct. The conditional impl would therefore affect types that can never use `Copy`. Use a generator proof and an ordinary derive for this change. [Rust Reference on bound checking](https://doc.rust-lang.org/reference/trait-bounds.html), [Rust compiler guide on trivial bounds](https://rustc-dev-guide.rust-lang.org/analysis/well-formed.html), [coherence and overlap](https://doc.rust-lang.org/reference/items/implementations.html#trait-implementation-coherence)


This is the minimal overlap probe. The two `Local` implementations compile when the conditional `Copy` impl is removed.

```rust
#[derive(Clone)]
struct Owned(String);

impl Copy for Owned where for<'a> String: Copy {}

trait Local {}
impl<T: Copy> Local for T {}
impl Local for Owned {} // E0119
```

**A rasn derive macro cannot query rustc's trait solver.** It receives the item as tokens, which may contain field paths to Rust definitions in other modules or crates. Adding generics to each generated struct would let Rust attach ordinary conditional trait bounds but would alter the public type signatures. Neither is necessary for types generated from linked inputs. [Proc-macro derive interface](https://doc.rust-lang.org/reference/procedural-macros.html#derive-macros), [Rust generic `Copy` behavior](https://doc.rust-lang.org/std/marker/trait.Copy.html#how-can-i-implement-copy)

## Verification and compatibility

Local compile probes confirmed that the four fixed-string and time representations satisfy `T: Copy`. A probe using the current compiler emitted `FixedOctetString<4>` for a named declaration, `OctetString` for the same length constraint on an inline field, and `Option<u8>` for an optional bounded integer.

- Compile generated bindings with a `T: Copy` assertion for an empty sequence, bounded signed and unsigned fields, optional and defaulted fields, nested anonymous structs, a `SET`, a `CHOICE`, and a forward reference through a cross-module alias chain. Include fixed-size wrappers and time types. Test `REAL` with an existing compatible derive configuration.
- Compile normal bindings and check derive output for the owning controls. Include unbounded and extensible integers, inline variable-storage strings, `Any`, collections of copyable elements, recursive types, and unavailable imports supplied by the caller. A cross-module test should also check that an explicit module reference cannot pick up the eligibility of a same-named declaration elsewhere.
- Check a representative encoding round trip and run the compiler workspace and snapshot suites. Preserve member types, attributes, tags, constraints, constructors, and default methods. Where opaque custom attributes alter trait or layout semantics, retain the caller's current handling and require proof before inserting a new derive.

A newly generated public `Copy` implementation is an API commitment. Future schema changes that add owning fields can remove that guarantee. Adding the derive can also conflict with downstream blanket implementations, manual `Copy` or `Drop` implementations in the crate that includes the generated code. Mention this in the release notes when enabling composite inference. [Rust `Copy` API guidance](https://doc.rust-lang.org/std/marker/trait.Copy.html#when-should-my-type-be-copy), [Rust trait coherence](https://doc.rust-lang.org/reference/items/implementations.html#trait-implementation-coherence)
