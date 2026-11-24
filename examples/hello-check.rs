use anyhow::{Context, Result, ensure};

fn main() -> Result<()> {
    let content = std::fs::read("hello.txt").context("read hello.txt in the candidate checkout")?;
    ensure!(
        content == b"hello\n",
        "expected hello.txt to contain hello followed by one newline"
    );
    Ok(())
}
