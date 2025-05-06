use include_dir::Dir;
use quickjs_rusty::{
    Context, ExecutionError, JsCompiledFunction, OwnedJsValue, ValueError,
    console::{ConsoleBackend, Level},
    serde::to_js,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::HashMap, fmt::Write};

#[cfg(feature = "with-axum")]
use axum::extract::FromRef;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error(transparent)]
    Serde(#[from] quickjs_rusty::serde::Error),
    #[error(transparent)]
    Context(#[from] quickjs_rusty::ContextError),

    #[cfg(feature = "transpiling")]
    #[error(transparent)]
    Parse(#[from] deno_ast::ParseDiagnostic),
    #[cfg(feature = "transpiling")]
    #[error(transparent)]
    Transpile(#[from] deno_ast::TranspileError),

    #[error("unexpected")]
    Unexpected(String),
}

#[cfg(feature = "with-axum")]
impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self {
            Error::Execution(quickjs_rusty::ExecutionError::Exception(msg)) => {
                log::error!("{:?}", msg.to_string());
                "Execution error".into_response()
            }
            err => {
                log::error!("{:?}", err);
                "Unhandled error".into_response()
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum Script {
    Function {
        args: Option<Value>,
        code: String,
    },
    #[cfg(feature = "transpiling")]
    RenderPage {
        args: Option<Value>,
        name: String,
    },
    CompiledFunction {
        args: Option<Value>,
        name: String,
    },
}

#[derive(Serialize, Debug)]
pub struct ScriptOutput {
    pub output: String,
    pub console_output: String,
}

#[derive(Debug)]
pub enum Function {
    Code(String),
    Compiled(JsCompiledFunction),
}

enum Message {
    ExecuteScript {
        script: Script,
        respond_to: tokio::sync::oneshot::Sender<Result<ScriptOutput, Error>>,
    },
}

static JS_SRC_DIR: OnceLock<Dir<'static>> = OnceLock::new();

pub struct RuntimeConfig<'a> {
    pub workers: usize,
    pub functions: Option<HashMap<String, String>>,
    pub js_src: Option<Dir<'a>>,
}

impl<'a> Default for RuntimeConfig<'a> {
    fn default() -> Self {
        Self {
            workers: 5,
            functions: Some(HashMap::new()),
            js_src: None,
        }
    }
}

#[derive(Clone)]
pub struct Runtime {
    sender: crossbeam::channel::Sender<Message>,
}

impl Runtime {
    pub fn new(config: RuntimeConfig<'static>) -> Self {
        if let Some(dir) = config.js_src {
            JS_SRC_DIR.get_or_init(|| dir);
        }

        let (sender, receiver) = crossbeam::channel::unbounded::<Message>();

        let functions = config.functions.unwrap_or_default();

        for i in 0..config.workers {
            let receiver = receiver.clone();
            let functions = functions.clone();
            Runtime::spawn_worker(receiver, functions)
        }

        Self { sender }
    }

    fn spawn_worker(
        receiver: crossbeam::channel::Receiver<Message>,
        functions: HashMap<String, String>,
    ) {
        std::thread::spawn(move || {
            log::debug!("spawn worker: {:?}", std::thread::current().id());

            let (context, compiled_fns) = context::init(functions)
                .map_err(|e| log::error!("failed to initialize runtime context: {}", e))
                .expect("Runtime context initialization failed");

            while let Ok(msg) = receiver.recv() {
                match msg {
                    Message::ExecuteScript { script, respond_to } => {
                        log::trace!("execute script");

                        let source = Runtime::prepare_script(script, &compiled_fns);

                        let msg = match source {
                            Ok((args, source)) => context::eval(&context, args, source),
                            Err(err) => Err(err),
                        };

                        _ = respond_to.send(msg);
                    }
                };
            }
        });
    }

    fn prepare_script(
        script: Script,
        compiled_fns: &HashMap<String, JsCompiledFunction>,
    ) -> Result<(Option<Value>, Function), Error> {
        match script {
            #[cfg(feature = "transpiling")]
            Script::RenderPage { args, name } => Ok((
                args,
                Function::Code(format!(
                    "globalThis.Pages.{0} ? globalThis.Pages.{0}(args): 'Page \"{0}\" not found'",
                    name
                )),
            )),
            Script::Function { args, code } => Ok((args, Function::Code(code))),
            Script::CompiledFunction { args, name } => {
                let function = compiled_fns
                    .get(&name)
                    .ok_or(Error::Unexpected(format!("function {} not found", name)))?
                    .to_owned();

                Ok((args, Function::Compiled(function)))
            }
        }
    }

    #[cfg(all(feature = "with-axum", feature = "transpiling"))]
    pub async fn render(
        &self,
        args: Option<Value>,
        page: &str,
    ) -> impl axum::response::IntoResponse {
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<ScriptOutput, Error>>();

        let msg = Message::ExecuteScript {
            script: Script::RenderPage {
                args,
                name: page.into(),
            },
            respond_to: sender,
        };

        _ = self.sender.send(msg);

        let res = receiver
            .await
            .map_err(|e| Error::Unexpected(e.to_string()))?;

        res.map(|res| axum::response::Html(res.output))
    }

    pub async fn execute_script(&self, script: Script) -> Result<ScriptOutput, Error> {
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<ScriptOutput, Error>>();

        let msg = Message::ExecuteScript {
            script,
            respond_to: sender,
        };

        _ = self.sender.send(msg);

        let res = receiver
            .await
            .map_err(|e| Error::Unexpected(e.to_string()))?;

        res
    }
}

#[cfg(feature = "with-axum")]
impl<AppState> axum::extract::FromRequestParts<AppState> for Runtime
where
    Self: FromRef<AppState>,
    AppState: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self::from_ref(state))
    }
}

pub mod context {
    use std::path::{Component, PathBuf};

    use super::*;

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

    pub fn init(
        functions: HashMap<String, String>,
    ) -> Result<(Context, HashMap<String, JsCompiledFunction>), Error> {
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

        context.run_module("/jsx-runtime")?;

        #[cfg(feature = "transpiling")]
        if JS_SRC_DIR
            .get()
            .map(|root| root.contains("pages/index.js"))
            .unwrap_or(false)
        {
            context.run_module("pages/index.js")?;
            //
        }

        // TODO
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

        Ok((context, compiled_fns))
    }

    fn module_loader(module_name: &str, opaque: *mut std::ffi::c_void) -> anyhow::Result<String> {
        log::trace!("module_loader: {module_name}");
        if module_name == "/jsx-runtime" {
            return Ok(include_str!("./js/jsx.js").into());
        }

        let dir = JS_SRC_DIR
            .get()
            .ok_or_else(|| anyhow::anyhow!("JS_SRC_DIR not initialized"))?;

        let file = dir
            .get_file(module_name)
            .ok_or_else(|| anyhow::anyhow!("Module {module_name} not found"))?;

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
        let normalized_module_name =
            if module_name.starts_with("./") || module_name.starts_with("../") {
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
}

#[cfg(feature = "transpiling")]
fn transpile(source: &str, ty: Option<deno_ast::MediaType>) -> Result<String, Error> {
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
fn transpile_jsx(source: &str) -> Result<String, Error> {
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
    use super::*;

    #[tokio::test]
    async fn sum() {
        let runtime = Runtime::new(RuntimeConfig::default());
        let res = runtime
            .execute_script(Script::Function {
                code: "console.log('test'); 1 + 1".into(),
                args: None,
            })
            .await
            .unwrap();

        assert_eq!(res.output, "2");
        assert_eq!(res.console_output, "test\n");

        let res = runtime
            .execute_script(Script::Function {
                code: "console.log('test2'); 2 + 2".into(),
                args: None,
            })
            .await
            .unwrap();

        assert_eq!(res.output, "4");
        assert_eq!(res.console_output, "test2\n");
    }

    #[tokio::test]
    async fn ctx() {
        let runtime = Runtime::new(RuntimeConfig::default());
        let res = runtime
            .execute_script(Script::Function {
                code: "let obj = {name: ctx.name, args}; JSON.stringify(obj);".into(),
                args: Some(json!(["a", "b"])),
            })
            .await
            .unwrap();

        assert_eq!(res.output, "{\"name\":\"script\",\"args\":[\"a\",\"b\"]}");
    }

    #[cfg(feature = "transpiling")]
    #[test]
    fn test_transpile_ts() {
        let source = "export type A = {args; any}; function a(args: A): {res: any} {};";
        assert_eq!(
            transpile(source.into(), None).unwrap(),
            "function a(args) {}\n"
        );
    }

    #[tokio::test]
    async fn compile() {
        let runtime = Runtime::new(RuntimeConfig {
            functions: Some(HashMap::from([("sum.js".into(), "args.a+args.b".into())])),
            ..Default::default()
        });
        let res = runtime
            .execute_script(Script::CompiledFunction {
                name: "sum.js".into(),
                args: Some(json!({"a": 1, "b": 1})),
            })
            .await
            .unwrap();

        assert_eq!(res.output, "2");
    }

    #[cfg(feature = "transpiling")]
    #[tokio::test]
    async fn compile_ts() {
        let runtime = Runtime::new(RuntimeConfig {
            functions: Some(HashMap::from([(
                "sum.ts".into(),
                r#"declare var args: { a: number; b: number };
                function sum(a: number, b: number): number {
                    const res = a + b;
                    console.log(`a + b = ${res}`);
                    return res;
                }
                sum(args.a, args.b);"#
                    .into(),
            )])),
            ..Default::default()
        });
        let res = runtime
            .execute_script(Script::CompiledFunction {
                name: "sum.ts".into(),
                args: Some(json!({"a": 1, "b": 1})),
            })
            .await
            .unwrap();

        assert_eq!(res.output, "2");
    }

    #[test]
    fn compile_example() {
        let context = Context::builder().build().unwrap();
        let js_context = unsafe { context.context_raw() };

        let source = "args.a + args.b";

        let compiled_fn = quickjs_rusty::compile::compile(js_context, &source, "test.js")
            .unwrap()
            .try_into_compiled_function()
            .unwrap();

        let args = to_js(js_context, &json!({"a": 1, "b": 1})).unwrap();
        context.set_global("args", args).unwrap();

        let res = compiled_fn.eval().unwrap().to_int().unwrap();

        assert_eq!(res, 2);

        let args = to_js(js_context, &json!({"a": 2, "b": 2})).unwrap();
        context.set_global("args", args).unwrap();

        let res = compiled_fn.eval().unwrap().to_int().unwrap();

        assert_eq!(res, 4);
    }

    #[cfg(feature = "transpiling")]
    #[tokio::test]
    async fn render_html() {
        let runtime = Runtime::new(RuntimeConfig::default());

        let res = runtime
            .execute_script(Script::RenderPage {
                args: Some(
                    json!({"items": [{"id": 1, "name": "first"}, {"id": 2, "name": "second"}]}),
                ),
                name:
                    "(props) => <div><ul>{props.items.map(({name}) => <li>{name}</li>)}</ul></div>"
                        .into(),
            })
            .await
            .unwrap();

        assert_eq!(
            res.output,
            "<div><ul><li>first</li><li>second</li></ul></div>"
        );
    }

    #[tokio::test]
    async fn pool() {
        unsafe {
            std::env::set_var("RUST_LOG", "debug");
        }
        env_logger::init();
        let runtime = Runtime::new(RuntimeConfig {
            workers: 2,
            ..Default::default()
        });

        let task1 = runtime.execute_script(Script::Function {
            args: None,
            code: "console.log('hello from first worker, loop forever'); while (true) {}".into(),
        });

        let task2 = async {
            let res = runtime
                .execute_script(Script::Function {
                    args: None,
                    code: "console.log('hello from second worker');".into(),
                })
                .await;
        };

        _ = tokio::time::timeout(std::time::Duration::from_millis(20), async {
            _ = tokio::join!(task1, task2);
        })
        .await;
    }

    #[test]
    fn example() {
        let console = context::Console::new();
        let output = console.output.clone();

        let context = Context::builder().console(console).build().unwrap();

        let value = context
            .eval("console.log('hello','world');console.log('!');1 + 2", false)
            .unwrap();
        println!("js: 1 + 1 = {:?}", value);

        let console_output = output.lock().unwrap();
        println!("{:?}", console_output);

        let context = context.reset().unwrap();

        let console = context::Console::new();
        let output = console.output.clone();

        _ = context.set_console(Box::new(console));

        let value = context.eval("console.log('!!!!!!');2 + 2", false).unwrap();
        println!("js: 2 + 2 = {:?}", value);

        let console_output = output.lock().unwrap();
        println!("{:?}", console_output);
    }
}
