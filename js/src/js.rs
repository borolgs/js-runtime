use quickjs_rusty::{
    Context, ExecutionError, JsCompiledFunction, OwnedJsValue,
    console::{ConsoleBackend, Level},
    serde::to_js,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, fmt::Write};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error(transparent)]
    Serde(#[from] quickjs_rusty::serde::Error),

    #[cfg(feature = "ts")]
    #[error(transparent)]
    Parse(#[from] deno_ast::ParseDiagnostic),
    #[cfg(feature = "ts")]
    #[error(transparent)]
    Transpile(#[from] deno_ast::TranspileError),

    #[error("unexpected")]
    Unexpected(String),
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum Script {
    Function { args: Option<Value>, code: String },
    CompiledFunction { args: Option<Value>, name: String },
}

#[derive(Debug)]
enum Function {
    Code(String),
    Compiled(JsCompiledFunction),
}

#[derive(Serialize, Debug)]
pub struct ScriptResult {
    pub output: String,
    pub console_output: String,
}

enum Message {
    ExecuteScript {
        script: Script,
        respond_to: tokio::sync::oneshot::Sender<Result<ScriptResult, Error>>,
    },
}

pub struct RuntimeConfig {
    pub workers: usize,
    pub functions: Option<HashMap<String, String>>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            workers: 5,
            functions: Some(HashMap::new()),
        }
    }
}

#[derive(Clone)]
pub struct Runtime {
    sender: crossbeam::channel::Sender<Message>,
}

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Self {
        let (sender, receiver) = crossbeam::channel::unbounded::<Message>();

        let functions = config.functions.unwrap_or_default();

        for i in 0..config.workers {
            let receiver = receiver.clone();
            let functions = functions.clone();
            std::thread::spawn(move || {
                log::info!("worker spawned: {:?}", std::thread::current().id());
                let context = Context::builder().build().unwrap();

                let js_context = unsafe { context.context_raw() };

                let ctx = to_js(js_context, &json!({"name": "script"})).unwrap();

                context.set_global("ctx", ctx).unwrap();

                let mut compiled_fns = HashMap::new();

                #[allow(unused_mut)]
                functions.into_iter().for_each(|(name, mut code)| {
                    if name.ends_with(".ts") {
                        #[cfg(feature = "ts")]
                        {
                            code = transpile_ts(&code).unwrap();
                        }

                        #[cfg(not(feature = "ts"))]
                        {
                            panic!(
                                "TypeScript is not supported. Enable the 'ts' feature to use it."
                            );
                        }
                    }
                    let compiled_fn = quickjs_rusty::compile::compile(js_context, &code, &name)
                        .unwrap()
                        .try_into_compiled_function()
                        .unwrap();

                    compiled_fns.insert(name, compiled_fn);
                });

                while let Ok(msg) = receiver.recv() {
                    match msg {
                        Message::ExecuteScript { script, respond_to } => {
                            let source = Runtime::prepare(script, &compiled_fns);

                            let msg = match source {
                                Ok((args, source)) => Runtime::eval(source, args, &context),
                                Err(err) => Err(err),
                            };

                            _ = respond_to.send(msg);
                        }
                    };
                }
            });
        }

        Self { sender }
    }

    fn prepare(
        script: Script,
        compiled_fns: &HashMap<String, JsCompiledFunction>,
    ) -> Result<(Option<Value>, Function), Error> {
        match script {
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

    fn eval(
        source: Function,
        args: Option<Value>,
        context: &Context,
    ) -> Result<ScriptResult, Error> {
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

        Ok(ScriptResult {
            output: result,
            console_output,
        })
    }

    pub async fn execute_script(&self, script: Script) -> Result<ScriptResult, Error> {
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<ScriptResult, Error>>();

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

struct Console {
    output: Arc<Mutex<String>>,
}

impl Console {
    fn new() -> Self {
        Self {
            output: Arc::new(Mutex::new(String::from(""))),
        }
    }
}

impl ConsoleBackend for Console {
    fn log(&self, _level: Level, values: Vec<OwnedJsValue>) {
        let output_line = values
            .into_iter()
            .map(|v| v.to_string().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ");
        log::debug!("{output_line}");
        let mut output = self.output.lock().unwrap();
        writeln!(output, "{}", output_line).unwrap();
    }
}

#[cfg(feature = "ts")]
fn transpile_ts(source: &str) -> Result<String, Error> {
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier: deno_ast::ModuleSpecifier::parse("test://script.ts").unwrap(),
        text: source.into(),
        media_type: deno_ast::MediaType::TypeScript,
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

    #[cfg(feature = "ts")]
    #[test]
    fn transpile() {
        let source = "export type A = {args; any}; function a(args: A): {res: any} {};";
        assert_eq!(
            transpile_ts(source.into()).unwrap(),
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

    #[cfg(feature = "ts")]
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
        let console = Console::new();
        let output = console.output.clone();

        let context = Context::builder().console(console).build().unwrap();

        let value = context
            .eval("console.log('hello','world');console.log('!');1 + 2", false)
            .unwrap();
        println!("js: 1 + 1 = {:?}", value);

        let console_output = output.lock().unwrap();
        println!("{:?}", console_output);

        let context = context.reset().unwrap();

        let console = Console::new();
        let output = console.output.clone();

        _ = context.set_console(Box::new(console));

        let value = context.eval("console.log('!!!!!!');2 + 2", false).unwrap();
        println!("js: 2 + 2 = {:?}", value);

        let console_output = output.lock().unwrap();
        println!("{:?}", console_output);
    }
}
