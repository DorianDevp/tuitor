// Two ways into the same service, which share its breakpoints: a controller and a
// command line command.
fn helper(n: u32) -> u32 {
    n * 2
}

fn service(n: u32) -> u32 {
    let doubled = helper(n);
    doubled + 1
}

fn controller() -> u32 {
    let input = 20;
    service(input)
}

fn command() -> u32 {
    let input = 30;
    service(input)
}

fn main() {
    let command_line = std::env::args().nth(1).as_deref() == Some("command");
    let result = if command_line { command() } else { controller() };
    println!("result={result}");
}
