use include_dir::{Dir, DirEntry};
use quickjs_rusty::{
    Context, JsCompiledFunction, OwnedJsValue,
    console::{ConsoleBackend, Level},
    serde::to_js,
};
use std::path::{Component, PathBuf};

use serde::Serialize;
use serde_json::json;
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::HashMap, fmt::Write};

static JS_SRC_DIR: OnceLock<Dir<'static>> = OnceLock::new();

use super::*;

pub struct ContextConfig<'a> {
    pub js_src: Option<Dir<'a>>,
}

impl<'a> Default for ContextConfig<'a> {
    fn default() -> Self {
        Self { js_src: None }
    }
}

#[derive(Debug)]
pub enum Function {
    Code(String),
    Compiled(JsCompiledFunction),
}

impl From<&str> for Function {
    fn from(value: &str) -> Self {
        Self::Code(value.into())
    }
}

pub struct Console {
    pub output: Arc<Mutex<String>>,
}

impl Console {
    pub fn new() -> Self {
        Self {
            output: Arc::new(Mutex::new(String::from(""))),
        }
    }
}

impl ConsoleBackend for Console {
    fn log(&self, _level: Level, values: Vec<OwnedJsValue>) {
        let output_line = values
            .into_iter()
            .map(|v| v.js_to_string().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ");
        log::debug!("{output_line}");
        let mut output = self.output.lock().unwrap();
        writeln!(output, "{}", output_line).unwrap();
    }
}

pub fn init_module_loader(config: ContextConfig<'static>) -> Option<&'static Dir<'static>> {
    if let Some(dir) = config.js_src {
        return Some(JS_SRC_DIR.get_or_init(|| dir));
    }

    None
}

pub fn get_js_dir() -> Option<&'static Dir<'static>> {
    JS_SRC_DIR.get()
}

pub fn init() -> Result<Context, Error> {
    let context = Context::builder().console(Console::new()).build()?;

    let js_context = unsafe { context.context_raw() };

    let ctx = to_js(js_context, &json!({"name": "script"}))?;

    context.set_global("ctx", ctx)?;

    let opaque: *mut std::ffi::c_void = std::ptr::null_mut();

    context.set_module_loader(
        Box::new(module_loader),
        Some(Box::new(module_normalize)),
        opaque,
    );

    Ok(context)
}

pub fn compile_functions(
    context: &Context,
    functions: HashMap<String, String>,
) -> Result<HashMap<String, JsCompiledFunction>, Error> {
    let js_context = unsafe { context.context_raw() };

    let mut compiled_fns = HashMap::new();

    #[allow(unused_mut)]
    for (name, mut code) in functions.into_iter() {
        if name.ends_with(".ts") {
            #[cfg(feature = "transpiling")]
            {
                code = transpile(&code, None)?;
            }

            #[cfg(not(feature = "transpiling"))]
            {
                panic!("TypeScript is not supported. Enable the 'ts' feature to use it.");
            }
        }
        let compiled_fn = quickjs_rusty::compile::compile(js_context, &code, &name)?
            .try_into_compiled_function()?;

        compiled_fns.insert(name, compiled_fn);
    }

    Ok(compiled_fns)
}

fn module_loader(module_name: &str, opaque: *mut std::ffi::c_void) -> anyhow::Result<String> {
    log::trace!("module_loader: {module_name}");

    // built-in
    if module_name == "/jsx-runtime" {
        return Ok(include_str!("./js/jsx.js").into());
    }

    let dir = JS_SRC_DIR
        .get()
        .ok_or_else(|| anyhow::anyhow!("JS_SRC_DIR not initialized"))?;

    let module = dir.get_entry(module_name);

    let file = match module {
        // try to get barrel file
        // TODO: handle .ts, .jsx, .tsx
        Some(DirEntry::Dir(dir)) => {
            if let Some(index) = dir.get_file("index.js") {
                Ok(index)
            } else {
                Err(anyhow::anyhow!("Module {module_name} not found"))
            }
        }
        Some(DirEntry::File(file)) => Ok(file),
        None => Err(anyhow::anyhow!("Module {module_name} not found")),
    }?;

    let source = file
        .contents_utf8()
        .ok_or_else(|| anyhow::anyhow!("Module {module_name} is not valid UTF-8"))?;

    if module_name.ends_with(".jsx") {
        #[cfg(feature = "transpiling")]
        return transpile_jsx(source).map_err(|e| anyhow::anyhow!(e));

        #[cfg(not(feature = "transpiling"))]
        return Err(anyhow::anyhow!(
            "JSX support requires the `transpiling` feature."
        ));
    }

    Ok(source.to_string())
}

fn module_normalize(
    module_base_name: &str,
    module_name: &str,
    opaque: *mut std::ffi::c_void,
) -> anyhow::Result<String> {
    let normalized_module_name = if module_name.starts_with("./") || module_name.starts_with("../")
    {
        let module_path = std::path::Path::new(module_base_name)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(module_name);

        let mut parts = Vec::new();

        for component in module_path.components() {
            match component {
                Component::ParentDir => {
                    if let Some(last) = parts.last() {
                        if *last != ".." {
                            parts.pop();
                            continue;
                        }
                    }
                    parts.push("..");
                }
                Component::CurDir => {}
                Component::Normal(p) => parts.push(p.to_str().unwrap()),
                Component::RootDir => parts.clear(),
                _ => {}
            }
        }

        parts
            .iter()
            .collect::<PathBuf>()
            .to_string_lossy()
            .into_owned()
    } else {
        module_name.to_string()
    };

    log::trace!(
        "module_normalize: '{}' + '{}' -> '{}'",
        module_base_name,
        module_name,
        normalized_module_name
    );

    Ok(normalized_module_name)
}

pub fn eval<Args>(
    context: &Context,
    args: Option<Args>,
    source: Function,
) -> Result<ScriptOutput, Error>
where
    Args: Serialize,
{
    let console = Console::new();
    let output = console.output.clone();

    context.set_console(Box::new(console))?;

    let js_context = unsafe { context.context_raw() };

    let args = to_js(js_context, &args)?;
    context.set_global("args", args)?;

    let result = match source {
        Function::Code(code) => context.eval(&code, false)?,
        Function::Compiled(compiled_fn) => compiled_fn.eval()?,
    };
    let result = result.js_to_string()?;

    let output = output.lock().unwrap();
    let console_output = output.clone();

    Ok(ScriptOutput {
        output: result,
        console_output,
    })
}

#[cfg(feature = "transpiling")]
pub fn transpile(source: &str, ty: Option<deno_ast::MediaType>) -> Result<String, Error> {
    let parsed = deno_ast::parse_script(deno_ast::ParseParams {
        specifier: deno_ast::ModuleSpecifier::parse("test://script.ts").unwrap(),
        text: source.into(),
        media_type: ty.unwrap_or(deno_ast::MediaType::TypeScript),
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })?;

    let res = parsed
        .transpile(
            &deno_ast::TranspileOptions {
                imports_not_used_as_values: deno_ast::ImportsNotUsedAsValues::Remove,
                use_decorators_proposal: true,
                ..Default::default()
            },
            &deno_ast::TranspileModuleOptions {
                ..Default::default()
            },
            &deno_ast::EmitOptions {
                source_map: deno_ast::SourceMapOption::Separate,
                inline_sources: true,
                ..Default::default()
            },
        )?
        .into_source();

    Ok(res.text)
}

#[cfg(feature = "transpiling")]
pub fn transpile_jsx(source: &str) -> Result<String, Error> {
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier: deno_ast::ModuleSpecifier::parse("test://script.ts").unwrap(),
        text: source.into(),
        media_type: deno_ast::MediaType::Jsx,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })?;

    let res = parsed
        .transpile(
            &deno_ast::TranspileOptions {
                imports_not_used_as_values: deno_ast::ImportsNotUsedAsValues::Remove,
                jsx_factory: "jsx".into(),
                jsx_automatic: true,
                ..Default::default()
            },
            &deno_ast::TranspileModuleOptions {
                ..Default::default()
            },
            &deno_ast::EmitOptions {
                source_map: deno_ast::SourceMapOption::Separate,
                inline_sources: true,
                ..Default::default()
            },
        )?
        .into_source();

    Ok(res.text)
}

#[cfg(test)]
mod tests {
    use include_dir::File;
    use serde_json::Value;

    use super::*;

    #[test]
    fn rum_module() {
        unsafe {
            std::env::set_var("RUST_LOG", "trace");
        }
        env_logger::init();

        let js_src = {
            let file = DirEntry::File(File::new(
                "lib.js",
                "globalThis.hello = 'hello';".as_bytes(),
            ));
            let files: &[DirEntry<'static>] = Box::leak(Box::new([file]));

            Dir::new("src", &files)
        };

        let dir = init_module_loader(ContextConfig {
            js_src: Some(js_src),
        });

        let ctx = init().unwrap();
        ctx.eval_module("import './lib.js';", false).unwrap();
        let res = context::eval(&ctx, Some(Value::Null), "globalThis.hello".into()).unwrap();

        assert_eq!(res.output, "hello");
    }
}
