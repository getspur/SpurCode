#[path = "../../src/runtime.rs"]
mod runtime;
use runtime::{RuntimeError, RuntimeOwner};
use std::{error::Error, io::Write, path::Path, sync::mpsc, thread};

fn main() -> Result<(), Box<dyn Error>> {
    let case = std::env::args().nth(1).ok_or("missing case")?;
    #[cfg(target_os = "macos")]
    if case == "native-loader-error" {
        let error = match RuntimeOwner::start_with_runtime_path(Path::new(
            "/private/tmp/spur-embedding-no-such-runtime.dylib",
        )) {
            Err(error) => error,
            Ok(_) => return Err("missing runtime unexpectedly initialized".into()),
        };
        assert!(matches!(error, RuntimeError::Initialization(_)));
        eprintln!("typed loader error: {error}");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if case == "init-error" {
        let error = match RuntimeOwner::start(Path::new("/dev/null/not-a-directory")) {
            Err(error) => error,
            Ok(_) => return Err("initialization unexpectedly succeeded".into()),
        };
        assert!(matches!(error, RuntimeError::Initialization(_)));
        eprintln!("typed initialization error: {error}");
        return Ok(());
    }
    let cache = std::env::var_os("SPUR_ORT_CACHE").ok_or("missing cache")?;
    let mut owner = RuntimeOwner::start(Path::new(&cache))?;
    if case == "wait-for-signal" {
        print!("R");
        std::io::stdout().flush()?;
        loop {
            thread::park();
        }
    }
    match case.as_str() {
        "active-close" => {
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            owner.submit(move || {
                let mut session = ort::session::Session::builder()
                    .unwrap()
                    .commit_from_memory(include_bytes!("../fixtures/mul_1.onnx"))
                    .unwrap();
                let input = ort::value::Tensor::from_array((
                    [3usize, 2],
                    vec![1.0_f32, 2., 3., 4., 5., 6.],
                ))
                .unwrap();
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                let outputs = session.run(ort::inputs![input]).unwrap();
                assert_eq!(
                    outputs[0].try_extract_tensor::<f32>().unwrap().1,
                    &[1., 4., 9., 16., 25., 36.]
                );
                drop(outputs);
                drop(session);
                eprintln!("phase: session_handle_dropped");
                result_tx.send(()).unwrap();
            })?;
            started_rx.recv()?;
            owner.close_with_hook(|| {
                release_tx.send(()).unwrap();
            })?;
            result_rx.recv()?;
            eprintln!("active native work joined before checked shutdown");
        }
        "inference" | "retained-output" => {
            let (out_tx, out_rx) = mpsc::channel();
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            owner.submit(move || {
                let mut session = ort::session::Session::builder()
                    .unwrap()
                    .commit_from_memory(include_bytes!("../fixtures/mul_1.onnx"))
                    .unwrap();
                let input = ort::value::Tensor::from_array((
                    [3usize, 2],
                    vec![1.0_f32, 2., 3., 4., 5., 6.],
                ))
                .unwrap();
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                let outputs = session.run(ort::inputs![input]).unwrap();
                let value = outputs.into_iter().next().unwrap().1;
                assert_eq!(
                    value.try_extract_tensor::<f32>().unwrap().1,
                    &[1., 4., 9., 16., 25., 36.]
                );
                drop(session);
                eprintln!("phase: session_handle_dropped");
                out_tx.send(value).unwrap();
            })?;
            started_rx.recv()?;
            release_tx.send(())?;
            let value = out_rx.recv()?;
            assert_eq!(
                value.try_extract_tensor::<f32>()?.1,
                &[1., 4., 9., 16., 25., 36.]
            );
            eprintln!("native output: {:?}", value.try_extract_tensor::<f32>()?.1);
            if case == "retained-output" {
                assert!(matches!(owner.close(), Err(RuntimeError::Busy(_))));
                assert!(matches!(owner.submit(|| {}), Err(RuntimeError::Closing)));
                eprintln!("checked close refused retained native output");
            }
            drop(value);
            eprintln!("phase: output_dropped");
        }
        "idle" | "repeated-close" | "reopen" => {}
        _ => return Err("unknown case".into()),
    }
    owner.close()?;
    eprintln!("checked native environment shutdown succeeded");
    if case == "repeated-close" {
        owner.close()?;
    }
    assert!(matches!(owner.submit(|| {}), Err(RuntimeError::Closing)));
    if case == "reopen" {
        assert!(
            matches!(
                RuntimeOwner::start(Path::new(&cache)),
                Err(RuntimeError::Initialization(_))
            ),
            "closed managed environment reopened"
        );
    }
    Ok(())
}
