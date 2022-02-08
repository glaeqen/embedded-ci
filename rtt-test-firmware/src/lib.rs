#![no_std]

use stm32f4xx_hal as hal; // memory layout
use panic_rtt_target as _;


/// Terminates the application and makes `probe-run` exit with exit-code = 0
pub fn exit() -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}
