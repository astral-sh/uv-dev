//! Parse selected upstream Rust functions into a closed, language-neutral AST.
//! Unsupported syntax is an error; no source-text replacement is used.
use quote::ToTokens;
use serde_json::{Value, json};
use std::{env, fs, path::Path};
use syn::{Expr, FnArg, ImplItem, Item, Pat, Stmt, Type, parse::Parser};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn unsupported<T>(node: &impl ToTokens) -> Result<T> {
    Err(format!("Unsupported Rust syntax: {}", node.to_token_stream()).into())
}

fn path(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn literal(lit: &syn::Lit) -> Result<Value> {
    Ok(match lit {
        syn::Lit::Str(v) => json!(v.value()),
        syn::Lit::Char(v) => json!(v.value().to_string()),
        syn::Lit::Byte(v) => json!(v.value()),
        syn::Lit::Int(v) => json!(v.base10_parse::<u64>()?),
        syn::Lit::Bool(v) => json!(v.value),
        _ => return unsupported(lit),
    })
}

fn pattern(pat: &Pat) -> Result<Value> {
    Ok(match pat {
        Pat::Ident(p) if p.ident == "None" && p.subpat.is_none() => {
            json!({"kind":"path","name":"None"})
        }
        Pat::Ident(p) if p.subpat.is_none() => json!({"kind":"bind","name":p.ident.to_string()}),
        Pat::Wild(_) => json!({"kind":"wild"}),
        Pat::Path(p) => json!({"kind":"path","name":path(&p.path)}),
        Pat::Lit(p) => json!({"kind":"literal","value":literal(&p.lit)?}),
        Pat::Or(p) => {
            json!({"kind":"or","items":p.cases.iter().map(pattern).collect::<Result<Vec<_>>>()?})
        }
        Pat::Tuple(p) => {
            json!({"kind":"tuple","items":p.elems.iter().map(pattern).collect::<Result<Vec<_>>>()?})
        }
        Pat::TupleStruct(p) => {
            json!({"kind":"constructor","name":path(&p.path),"items":p.elems.iter().map(pattern).collect::<Result<Vec<_>>>()?})
        }
        Pat::Struct(p) => {
            json!({"kind":"struct","name":path(&p.path),"fields":p.fields.iter().map(|f|Ok((f.member.to_token_stream().to_string(),pattern(&f.pat)?))).collect::<Result<serde_json::Map<_,_>>>()?,"rest":p.rest.is_some()})
        }
        Pat::Slice(p) => {
            json!({"kind":"slice","items":p.elems.iter().map(pattern).collect::<Result<Vec<_>>>()?})
        }
        Pat::Reference(p) => pattern(&p.pat)?,
        Pat::Type(p) => pattern(&p.pat)?,
        Pat::Paren(p) => pattern(&p.pat)?,
        Pat::Range(p) => {
            json!({"kind":"range","start":p.start.as_deref().map(expr).transpose()?,"end":p.end.as_deref().map(expr).transpose()?,"inclusive":matches!(p.limits,syn::RangeLimits::Closed(_))})
        }
        _ => return unsupported(pat),
    })
}

fn macro_expr(mac: &syn::Macro) -> Result<Value> {
    let name = path(&mac.path);
    if name == "vec" && mac.tokens.to_string().contains(';') {
        let tokens = &mac.tokens;
        if let Expr::Repeat(repeat) = syn::parse2::<Expr>(quote::quote!([#tokens]))? {
            return Ok(
                json!({"kind":"repeat","value":expr(&repeat.expr)?,"count":expr(&repeat.len)?}),
            );
        }
    }
    if name == "matches" {
        let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<(Expr, Pat)> {
            let value = input.parse()?;
            input.parse::<syn::Token![,]>()?;
            let pat = Pat::parse_multi(input)?;
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
            Ok((value, pat))
        };
        let (value, pat) = parser.parse2(mac.tokens.clone())?;
        return Ok(json!({"kind":"matches","value":expr(&value)?,"pattern":pattern(&pat)?}));
    }
    match name.as_str() {
        "format" | "write" | "writeln" | "vec" | "warn" | "debug" | "trace" | "warn_user_once" => {
            let args = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(mac.tokens.clone())?;
            Ok(
                json!({"kind":"macro","name":name,"args":args.iter().map(expr).collect::<Result<Vec<_>>>()?}),
            )
        }
        _ => unsupported(mac),
    }
}

fn expr(value: &Expr) -> Result<Value> {
    Ok(match value {
        Expr::Lit(e) => json!({"kind":"literal","value":literal(&e.lit)?}),
        Expr::Path(e) if e.qself.is_none() => json!({"kind":"path","name":path(&e.path)}),
        Expr::Reference(e) => expr(&e.expr)?,
        Expr::Paren(e) => expr(&e.expr)?,
        Expr::Group(e) => expr(&e.expr)?,
        Expr::Try(e) => json!({"kind":"try","value":expr(&e.expr)?}),
        Expr::Field(e) => {
            json!({"kind":"field","value":expr(&e.base)?,"field":e.member.to_token_stream().to_string()})
        }
        Expr::MethodCall(e) => {
            json!({"kind":"method","value":expr(&e.receiver)?,"name":e.method.to_string(),"generic":e.turbofish.as_ref().map(|v|v.to_token_stream().to_string()),"args":e.args.iter().map(expr).collect::<Result<Vec<_>>>()?})
        }
        Expr::Call(e) => {
            json!({"kind":"call","function":expr(&e.func)?,"args":e.args.iter().map(expr).collect::<Result<Vec<_>>>()?})
        }
        Expr::Binary(e) => {
            json!({"kind":"binary","operator":e.op.to_token_stream().to_string(),"left":expr(&e.left)?,"right":expr(&e.right)?})
        }
        Expr::Unary(e) => {
            json!({"kind":"unary","operator":e.op.to_token_stream().to_string(),"value":expr(&e.expr)?})
        }
        Expr::Assign(e) => json!({"kind":"assign","left":expr(&e.left)?,"right":expr(&e.right)?}),
        Expr::Block(e) if e.label.is_none() => json!({"kind":"block","body":block(&e.block)?}),
        Expr::If(e) => {
            json!({"kind":"if","condition":expr(&e.cond)?,"then":block(&e.then_branch)?,"else":e.else_branch.as_ref().map(|(_,e)|expr(e)).transpose()?})
        }
        Expr::Let(e) => json!({"kind":"let","pattern":pattern(&e.pat)?,"value":expr(&e.expr)?}),
        Expr::ForLoop(e) if e.label.is_none() => {
            json!({"kind":"for","pattern":pattern(&e.pat)?,"value":expr(&e.expr)?,"body":block(&e.body)?})
        }
        Expr::While(e) if e.label.is_none() => {
            json!({"kind":"while","condition":expr(&e.cond)?,"body":block(&e.body)?})
        }
        Expr::Break(e) if e.label.is_none() && e.expr.is_none() => json!({"kind":"break"}),
        Expr::Continue(e) if e.label.is_none() => json!({"kind":"continue"}),
        Expr::Match(e) => {
            json!({"kind":"match","value":expr(&e.expr)?,"arms":e.arms.iter().map(|a| {
            Ok(json!({"pattern":pattern(&a.pat)?,"body":expr(&a.body)?,"guard":a.guard.as_ref().map(|(_,guard)|expr(guard)).transpose()?}))
        }).collect::<Result<Vec<_>>>()?})
        }
        Expr::Return(e) => {
            json!({"kind":"return","value":e.expr.as_deref().map(expr).transpose()?})
        }
        Expr::Closure(e) if e.asyncness.is_none() && e.lifetimes.is_none() => {
            json!({"kind":"closure","params":e.inputs.iter().map(pattern).collect::<Result<Vec<_>>>()?,"body":expr(&e.body)?})
        }
        Expr::Struct(e) if e.rest.is_none() => {
            json!({"kind":"struct","name":path(&e.path),"fields":e.fields.iter().map(|f|Ok((f.member.to_token_stream().to_string(),expr(&f.expr)?))).collect::<Result<serde_json::Map<_,_>>>()?})
        }
        Expr::Tuple(e) => {
            json!({"kind":"tuple","items":e.elems.iter().map(expr).collect::<Result<Vec<_>>>()?})
        }
        Expr::Array(e) => {
            json!({"kind":"array","items":e.elems.iter().map(expr).collect::<Result<Vec<_>>>()?})
        }
        Expr::Index(e) => json!({"kind":"index","value":expr(&e.expr)?,"index":expr(&e.index)?}),
        Expr::Range(e) => {
            json!({"kind":"range","start":e.start.as_deref().map(expr).transpose()?,"end":e.end.as_deref().map(expr).transpose()?,"inclusive":matches!(e.limits,syn::RangeLimits::Closed(_))})
        }
        Expr::Cast(e) => {
            json!({"kind":"cast","value":expr(&e.expr)?,"type":e.ty.to_token_stream().to_string()})
        }
        Expr::Macro(e) => macro_expr(&e.mac)?,
        _ => return unsupported(value),
    })
}

fn function(sig: &syn::Signature, body: &syn::Block) -> Result<Value> {
    if sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
    {
        return unsupported(sig);
    }
    let params = sig
        .inputs
        .iter()
        .map(|arg| match arg {
            FnArg::Receiver(_) => Ok(json!({"kind":"bind","name":"self"})),
            FnArg::Typed(arg) => pattern(&arg.pat),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"name":sig.ident.to_string(),"params":params,"body":block(body)?}))
}

fn block(body: &syn::Block) -> Result<Vec<Value>> {
    body.stmts
        .iter()
        .map(|stmt| {
            Ok(match stmt {
                Stmt::Local(local) => {
                    let Some(init) = &local.init else {
                        return unsupported(local);
                    };
                    json!({"kind":"local","pattern":pattern(&local.pat)?,"value":expr(&init.expr)?,"else":init.diverge.as_ref().map(|(_,value)|expr(value)).transpose()?})
                }
                Stmt::Expr(e, semi) => {
                    json!({"kind":"expression","value":expr(e)?,"tail":semi.is_none()})
                }
                Stmt::Item(Item::Fn(f)) => {
                    json!({"kind":"function","value":function(&f.sig,&f.block)?})
                }
                Stmt::Macro(m) => {
                    json!({"kind":"expression","value":macro_expr(&m.mac)?,"tail":false})
                }
                _ => return unsupported(stmt),
            })
        })
        .collect()
}

fn type_name(ty: &Type) -> Result<String> {
    match ty {
        Type::Path(p) if p.qself.is_none() => Ok(path(&p.path)),
        _ => unsupported(ty),
    }
}

fn main() -> Result<()> {
    let root = env::args()
        .nth(1)
        .ok_or("Usage: uv-build-slim-codegen VENDOR_DIRECTORY")?;
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(Path::new(&root).join("manifest.json"))?)?;
    let mut functions = serde_json::Map::new();
    let mut structs = serde_json::Map::new();
    let mut errors = serde_json::Map::new();
    let mut variants = serde_json::Map::new();
    for file in manifest["files"]
        .as_array()
        .ok_or("files must be an array")?
    {
        let name = file["path"].as_str().ok_or("missing source path")?;
        let source = fs::read_to_string(Path::new(&root).join(name))?;
        let ast = syn::parse_file(&source)?;
        let selected = file["functions"]
            .as_array()
            .ok_or("missing selected functions")?
            .iter()
            .map(|s| s.as_str().ok_or("function must be a string"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let selected_structs = file["structs"]
            .as_array()
            .ok_or("missing selected structs")?
            .iter()
            .map(|s| s.as_str().ok_or("struct must be a string"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for item in &ast.items {
            match item {
                Item::Fn(f) if selected.contains(&f.sig.ident.to_string().as_str()) => {
                    functions.insert(f.sig.ident.to_string(), function(&f.sig, &f.block)?);
                }
                Item::Impl(implementation) => {
                    let owner = type_name(&implementation.self_ty)?;
                    for item in &implementation.items {
                        if let ImplItem::Fn(f) = item {
                            let key = format!("{owner}::{}", f.sig.ident);
                            if selected.contains(&key.as_str()) {
                                let mut func = function(&f.sig, &f.block)
                                    .map_err(|err| format!("{key}: {err}"))?;
                                func["owner"] = json!(owner);
                                functions.insert(key, func);
                            }
                        }
                    }
                }
                Item::Struct(s) if selected_structs.contains(&s.ident.to_string().as_str()) => {
                    let fields = s.fields.iter().enumerate().map(|(index,field)| {
                        json!({"name":field.ident.as_ref().map_or_else(||index.to_string(),ToString::to_string),"type":field.ty.to_token_stream().to_string()})
                    }).collect::<Vec<_>>();
                    structs.insert(s.ident.to_string(), json!(fields));
                }
                Item::Enum(e) => {
                    for variant in &e.variants {
                        variants.insert(
                            format!("{}::{}", e.ident, variant.ident),
                            json!(
                                variant
                                    .fields
                                    .iter()
                                    .enumerate()
                                    .map(|(index, field)| field
                                        .ident
                                        .as_ref()
                                        .map_or_else(|| index.to_string(), ToString::to_string))
                                    .collect::<Vec<_>>()
                            ),
                        );
                        for attr in &variant.attrs {
                            if attr.path().is_ident("error") {
                                let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
                                if let Ok(arguments) = attr.parse_args_with(parser)
                                    && let Some(Expr::Lit(lit)) = arguments.first()
                                    && let syn::Lit::Str(message) = &lit.lit
                                {
                                    errors.insert(
                                        format!("{}::{}", e.ident, variant.ident),
                                        json!(message.value()),
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for selected in selected {
            if !functions.contains_key(selected) {
                return Err(format!("Missing selected Rust function {selected} in {name}").into());
            }
        }
        for selected in selected_structs {
            if !structs.contains_key(selected) {
                return Err(format!("Missing selected Rust struct {selected} in {name}").into());
            }
        }
    }
    // Rust and Python may use different Unicode versions. Generate Rust's actual table.
    let mut alphanumeric = Vec::new();
    let mut start = None;
    for code in 0..=0x110000 {
        let accepted = char::from_u32(code).is_some_and(char::is_alphanumeric);
        if accepted && start.is_none() {
            start = Some(code);
        }
        if !accepted && let Some(first) = start.take() {
            alphanumeric.push((first, code - 1));
        }
    }
    let alphabetic: Vec<_> = alphanumeric
        .iter()
        .flat_map(|(start, end)| *start..=*end)
        .filter(|code| char::from_u32(*code).is_some_and(char::is_alphabetic))
        .collect();
    let mut alphabetic_ranges = Vec::new();
    for code in alphabetic {
        if let Some((_, end)) = alphabetic_ranges.last_mut()
            && *end + 1 == code
        {
            *end = code;
        } else {
            alphabetic_ranges.push((code, code));
        }
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"functions":functions,"structs":structs,"errors":errors,"variants":variants,"alphanumeric":alphanumeric,"alphabetic":alphabetic_ranges})
        )?
    );
    Ok(())
}
