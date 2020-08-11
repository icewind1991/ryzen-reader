# ryzer-reader

Read cpu power usage from AMD ryzen cpus on linux

## Example

```rust
use ryzen_reader::{CpuInfo, Error};

fn main() -> Result<(), Error> {
    let cpu = CpuInfo::new()?;
    let power = cpu.read()?;

    println!("Package power:");
    for (package, usage) in power.packages().enumerate() {
        println!("\t#{}: {:.2}W", package, usage);
    }
    println!("Core power:");
    for (core, usage) in power.cores().enumerate() {
        println!("\t#{}: {:.2}W", core, usage);
    }
    Ok(())
}
```

## Credits

This code is largely based on [rapl-read-ryzen](https://github.com/djselbeck/rapl-read-ryzen) and all credits on how to read the values goes to it's author.