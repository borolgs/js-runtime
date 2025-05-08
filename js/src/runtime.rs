use crate::{
    Error,
    context::{self, Function},
};
use include_dir::Dir;
use quickjs_rusty::JsCompiledFunction;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[cfg(feature = "with-axum")]
use axum::extract::FromRef;

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

enum Message {
    ExecuteScript {
        script: Script,
        respond_to: tokio::sync::oneshot::Sender<Result<ScriptOutput, Error>>,
    },
}

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
        context::init_module_loader(context::ContextConfig {
            js_src: config.js_src,
        });

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

            let context = context::init()
                .map_err(|e| log::error!("failed to initialize runtime context: {}", e))
                .expect("Runtime context initialization failed");

            let mut compiled_fns = context::compile_functions(&context, functions).unwrap();

            let page_fns = Runtime::init_jsx_renderer(&context).unwrap();

            compiled_fns.extend(page_fns.into_iter());

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

    fn init_jsx_renderer(
        context: &quickjs_rusty::Context,
    ) -> Result<HashMap<String, JsCompiledFunction>, Error> {
        context.run_module("/jsx-runtime")?;

        let js_context = unsafe { context.context_raw() };

        let mut compiled_fns = HashMap::new();

        #[cfg(feature = "transpiling")]
        if let Some(pages_dir) = context::get_js_dir()
            .map(|root| root.get_dir("pages"))
            .flatten()
        {
            log::debug!("Found 'pages' dir, initiating page renderers...");

            let pages = pages_dir
                .files()
                .map(|page| {
                    let name = page.path().file_stem().unwrap().to_str().unwrap();
                    let ext = page.path().extension().unwrap().to_str().unwrap();
                    (name, ext)
                })
                .filter(|(_, e)| *e == "jsx")
                .collect::<Vec<_>>();

            let imports = pages
                .iter()
                .map(|(name, ext)| format!("import {0} from 'pages/{0}.{1}'", name, ext))
                .collect::<Vec<_>>()
                .join("\n");

            let names = pages
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");

            let index = format!("{}\nglobalThis.__pages = {{ {} }};", imports, names);

            let res = context.eval_module(&index, false);

            for (name, _) in pages {
                let compiled_fn = quickjs_rusty::compile::compile(
                    js_context,
                    &format!("globalThis.__pages.{}(args);", name),
                    name,
                )?
                .try_into_compiled_function()?;

                compiled_fns.insert(name.to_string(), compiled_fn);
            }
        }

        Ok(compiled_fns)
    }

    fn prepare_script(
        script: Script,
        compiled_fns: &HashMap<String, JsCompiledFunction>,
    ) -> Result<(Option<Value>, Function), Error> {
        match script {
            #[cfg(feature = "transpiling")]
            // Script::RenderPage { args, name } => Ok((args, Function::Compiled(name))),
            Script::Function { args, code } => Ok((args, Function::Code(code))),
            Script::RenderPage { args, name } | Script::CompiledFunction { args, name } => {
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

#[cfg(test)]
mod tests {
    use quickjs_rusty::{Context, serde::to_js};
    use serde_json::json;

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
            context::transpile(source.into(), None).unwrap(),
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
