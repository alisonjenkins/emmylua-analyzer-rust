use clap::Parser;
use emmylua_check::{cmd_args::CmdArgs, run_check};
use emmylua_code_analysis::{WORKER_STACK_BYTES, register_thread_stack};
use mimalloc::MiMalloc;
use std::error::Error;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let cmd_args = CmdArgs::parse();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_BYTES)
        .on_thread_start(|| register_thread_stack(WORKER_STACK_BYTES))
        .build()?
        .block_on(run_check(cmd_args))
}
