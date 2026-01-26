use koto::{
    Koto,
    runtime::{DisplayContext, KValue, KotoVm},
};

fn format_kvalue(value: &KValue) -> anyhow::Result<String> {
    let mut ctx = DisplayContext::default();
    value
        .display(&mut ctx)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(ctx.result())
}

fn main() -> anyhow::Result<()> {
    let mut koto = Koto::default();
    let source = "let x = 42; || 1 + 1234";
    let chunk = koto.compile(source)?;
    let value = koto.run(chunk)?;
    println!(
        "regular function: {:?}",
        format_kvalue(&koto.call_function(value.clone(), &[])?)?
    );

    let (ip, bytes, constants) = match value {
        koto::runtime::KValue::Function(f) => {
            println!("{} {} {}", f.ip, f.arg_count, f.optional_arg_count);
            println!(
                "flags: {} {} {} {}",
                f.flags.is_variadic(),
                f.flags.is_generator(),
                f.flags.arg_is_unpacked_tuple(),
                f.flags.non_local_access(),
            );
            match f.context {
                Some(ctx) => println!(
                    "context: {:?} {:?}",
                    ctx.captures
                        .as_ref()
                        .map(|l| format_kvalue(&l.clone().into())),
                    ctx.non_locals.as_ref().map(|_| todo!("non_locals")),
                ),
                None => println!("context: None"),
            }
            println!("chunk.constants: {:?}", f.chunk.constants);
            println!("chunk.path: {:?}", f.chunk.path);
            println!("chunk.debug_info: {:?}", f.chunk.path);
            (f.ip, f.chunk.bytes.clone(), f.chunk.constants.clone())
        }
        _ => anyhow::bail!("not a function"),
    };

    let chunk2 = koto::bytecode::Chunk {
        bytes,
        constants,
        path: None,
        debug_info: koto::bytecode::DebugInfo::default(),
    };
    let function2 = koto::runtime::KFunction::new(
        /* chunk: */ koto::runtime::Ptr::from(chunk2),
        /* ip: */ ip,
        /* arg_count: */ 0,
        /* optional_arg_count: */ 0,
        /* flags: */
        koto::bytecode::FunctionFlags::new(
            /* variadic: */ false, /* generator: */ false,
            /* arg_is_unpacked_tuple: */ false, /* non_local_access: */ false,
        ),
        /* context: */ None,
    );

    let mut value2 = koto.call_function(koto::runtime::KValue::Function(function2), &[])?;
    println!("copied function: {value2:?}");

    while let koto::runtime::KValue::Function(f3) = value2 {
        value2 = koto.call_function(koto::runtime::KValue::Function(f3), &[])?;
        println!("copied function (again): {value2:?}");
    }

    Ok(())
}
