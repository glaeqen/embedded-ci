#![no_main]
#![no_std]

use defmt_test_firmware as _; // global logger + panicking-behavior + memory layout

#[rtic::app(device = stm32f4xx_hal::pac, dispatchers = [WWDG])]
mod app {
    use defmt_test_firmware::exit;
    use systick_monotonic::*;

    #[monotonic(binds = SysTick, default = true)]
    type Mono = Systick<1_000>;

    // Shared resources go here
    #[shared]
    struct Shared {}

    // Local resources go here
    #[local]
    struct Local {}

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        defmt::info!("init");

        task1::spawn().ok();

        // Setup the monotonic timer
        (
            Shared {
                // Initialization of shared resources go here
            },
            Local {
                // Initialization of local resources go here
            },
            init::Monotonics(Systick::new(cx.core.SYST, 16_000_000)),
        )
    }

    // Optional idle, can be removed if not needed.
    #[idle]
    fn idle(_: idle::Context) -> ! {
        defmt::info!("idle");

        loop {
            continue;
        }
    }

    // TODO: Add tasks
    #[task(local = [cnt: u32 = 0])]
    fn task1(cx: task1::Context) {
        defmt::info!("Hello from task1!");
        defmt::error!("Error from task1!");
        defmt::println!("Println from task1!");

        *cx.local.cnt += 1;

        // if *cx.local.cnt == 2 {
        //     unsafe {
        //         core::ptr::read_volatile(0xFFFFFFF as *const u32);
        //     }
        // }

        if *cx.local.cnt > 3 {
            exit();
        }

        task1::spawn_after(1.secs()).ok();
    }
}
