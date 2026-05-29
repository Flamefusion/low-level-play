use gpiod::{Chip, Options, RequestFlags};
use std::thread::sleep;
use std::time::Duration;

/// Deasserts (sets HIGH) the hardware RESET pins on the MCP chips
pub fn initialize_mcp_resets() -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing MCP hardware resets...");

    // Open gpiochip0 (the primary RP1 GPIO expander on Pi 5)
    let chip = Chip::new("/dev/gpiochip0")?;

    // We want to control the GPIO lines 5, 6, 12, 13
    let lines = [5, 6, 12, 13];

    // Request all reset lines as output lines
    let mut options = Options::new();
    options.direction(gpiod::Direction::Output);

    let requested = chip.request_lines(&lines, &options)?;

    // Pull reset LOW for 10 milliseconds to force a physical hardware reset
    requested.set_values(&[0, 0, 0, 0])?;
    sleep(Duration::from_millis(10));

    // Pull reset HIGH to release the MCP chips and boot them up!
    requested.set_values(&[1, 1, 1, 1])?;
    println!("MCP Chips successfully pulled out of Hardware Reset!");

    // Important: To prevent the line from falling back to low when the program finishes,
    // in real deployments, these lines must remain configured or held high.
    // In this basic script, we keep them in scope or let standard pull-ups handle it.

    Ok(())
}
