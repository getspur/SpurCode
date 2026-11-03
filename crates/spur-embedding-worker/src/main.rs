mod model_artifacts;
mod models;
mod runtime;
use runtime::RuntimeOwner;
use spur_embedding::{
    read_frame, write_frame, ErrorCode, ModelId, ProtocolLimits, Reply, ReplyEnvelope, Request,
    RequestEnvelope, Session, PROTOCOL_VERSION,
};
use std::{
    error::Error,
    io::{self, Read, Write},
    path::PathBuf,
};

fn parse_limits(args: &[String]) -> Result<ProtocolLimits, Box<dyn Error>> {
    let values: Vec<usize> = args.iter().map(|x| x.parse()).collect::<Result<_, _>>()?;
    let [frame, texts, text_bytes, total_bytes, dimensions, identity_bytes, pending] =
        values.as_slice()
    else {
        return Err("expected seven explicit protocol limits".into());
    };
    let limits = ProtocolLimits {
        max_frame_bytes: u32::try_from(*frame)?,
        max_texts: *texts,
        max_text_bytes: *text_bytes,
        max_total_text_bytes: *total_bytes,
        max_dimensions: *dimensions,
        max_identity_bytes: *identity_bytes,
        max_pending: *pending,
    };
    limits.validate()?;
    Ok(limits)
}

fn reply_error(
    output: &mut impl Write,
    limits: &ProtocolLimits,
    generation: uuid::Uuid,
    request_id: u64,
    code: ErrorCode,
    message: &str,
) -> Result<(), Box<dyn Error>> {
    let response = ReplyEnvelope {
        version: PROTOCOL_VERSION,
        generation,
        request_id,
        reply: Reply::Error {
            code,
            message: message.chars().take(limits.max_identity_bytes).collect(),
        },
    };
    write_frame(&mut *output, &response, limits)?;
    output.flush()?;
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--describe-model") {
        if args.len() != 12 {
            return Err("expected model, budget, correlation, and seven protocol limits".into());
        }
        let generation: uuid::Uuid = args[3].parse()?;
        let request_id: u64 = args[4].parse()?;
        let limits = parse_limits(&args[5..])?;
        let mut output = io::stdout().lock();
        if generation.is_nil() || request_id == 0 {
            return Err("invalid correlation".into());
        }
        let budget: u64 = match args[2].parse() {
            Ok(value) => value,
            Err(_) => {
                reply_error(
                    &mut output,
                    &limits,
                    generation,
                    request_id,
                    ErrorCode::InvalidRequest,
                    "invalid artifact budget",
                )?;
                return Ok(());
            }
        };
        if args[1] != "NomicEmbedTextV15" {
            reply_error(
                &mut output,
                &limits,
                generation,
                request_id,
                ErrorCode::Unavailable,
                "model unavailable",
            )?;
            return Ok(());
        }
        let resolved = match model_artifacts::resolve(budget) {
            Ok(value) => value,
            Err(_) => {
                reply_error(
                    &mut output,
                    &limits,
                    generation,
                    request_id,
                    ErrorCode::Unavailable,
                    "model artifacts unavailable",
                )?;
                return Ok(());
            }
        };
        if resolved.identity.validate(&limits).is_err() {
            reply_error(
                &mut output,
                &limits,
                generation,
                request_id,
                ErrorCode::InvalidRequest,
                "identity exceeds policy",
            )?;
            return Ok(());
        }
        let response = ReplyEnvelope {
            version: PROTOCOL_VERSION,
            generation,
            request_id,
            reply: Reply::Ready {
                identity: resolved.identity,
            },
        };
        write_frame(&mut output, &response, &limits)?;
        output.flush()?;
        return Ok(());
    }
    if args.len() != 8 {
        return Err("expected seven protocol limits and artifact budget".into());
    }
    let limits = parse_limits(&args[..7])?;
    let budget: u64 = args[7].parse()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let hello: RequestEnvelope = read_frame(&mut input, &limits)?;
    let Request::Hello { identity } = &hello.request else {
        return Err("expected Hello".into());
    };
    let mut session = Session::new(hello.generation, identity.clone(), limits.clone())?;
    session.validate_request(&hello)?;
    if identity.model != ModelId::NomicEmbedTextV15 {
        reply_error(
            &mut output,
            &limits,
            hello.generation,
            hello.request_id,
            ErrorCode::Unavailable,
            "model unavailable",
        )?;
        return Ok(());
    }
    let resolved = match model_artifacts::resolve(budget) {
        Ok(value) => value,
        Err(_) => {
            reply_error(
                &mut output,
                &limits,
                hello.generation,
                hello.request_id,
                ErrorCode::Unavailable,
                "model artifacts unavailable",
            )?;
            return Ok(());
        }
    };
    if *identity != resolved.identity {
        reply_error(
            &mut output,
            &limits,
            hello.generation,
            hello.request_id,
            ErrorCode::InvalidRequest,
            "model identity mismatch",
        )?;
        return Ok(());
    }
    let cache = match std::env::var_os("SPUR_ORT_CACHE") {
        Some(path) => PathBuf::from(path),
        None => {
            reply_error(
                &mut output,
                &limits,
                hello.generation,
                hello.request_id,
                ErrorCode::Unavailable,
                "native initialization failed",
            )?;
            return Err("SPUR_ORT_CACHE is required".into());
        }
    };
    let mut owner = match RuntimeOwner::start(&cache) {
        Ok(owner) => owner,
        Err(error) => {
            let response = ReplyEnvelope {
                version: PROTOCOL_VERSION,
                generation: hello.generation,
                request_id: hello.request_id,
                reply: Reply::Error {
                    code: ErrorCode::Unavailable,
                    message: "native initialization failed"
                        .chars()
                        .take(limits.max_identity_bytes)
                        .collect(),
                },
            };
            session.complete_reply(&hello, &response)?;
            write_frame(&mut output, &response, &limits)?;
            output.flush()?;
            return Err(error.into());
        }
    };
    let mut model_cache = models::ModelCache::new(resolved);
    let result = serve(
        &limits,
        &mut owner,
        &mut model_cache,
        &mut input,
        &mut output,
        session,
        hello,
    );
    let close = owner.close_after_join(|| model_cache.clear());
    close?;
    result
}
fn serve(
    limits: &ProtocolLimits,
    owner: &mut RuntimeOwner,
    model_cache: &mut models::ModelCache,
    input: &mut impl Read,
    output: &mut impl Write,
    mut session: Session,
    hello: RequestEnvelope,
) -> Result<(), Box<dyn Error>> {
    let mut first = Some(hello);
    loop {
        let request = if let Some(hello) = first.take() {
            hello
        } else {
            let request = read_frame(&mut *input, limits)?;
            session.validate_request(&request)?;
            request
        };
        let reply = match &request.request {
            Request::Hello { identity } => Reply::Ready {
                identity: identity.clone(),
            },
            Request::Embed { identity, texts } => match model_cache.embed(texts) {
                Ok(rows) => Reply::Embeddings {
                    identity: identity.clone(),
                    rows,
                },
                Err(_) => Reply::Error {
                    code: ErrorCode::Unavailable,
                    message: "model inference failed"
                        .chars()
                        .take(limits.max_identity_bytes)
                        .collect(),
                },
            },
            Request::Shutdown => {
                owner.close_after_join(|| model_cache.clear())?;
                Reply::Closed
            }
        };
        let shutdown = matches!(reply, Reply::Closed);
        let response = ReplyEnvelope {
            version: PROTOCOL_VERSION,
            generation: request.generation,
            request_id: request.request_id,
            reply,
        };
        session.complete_reply(&request, &response)?;
        write_frame(&mut *output, &response, limits)?;
        output.flush()?;
        if shutdown {
            return Ok(());
        }
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    run()
}
