//! This example demonstrates a poll-loop problem with many primitives in Embassy.
//!
//! Any primitive that uses WakerRegistration will have a problem when there is more than a
//! single task that waits.  The comments within WakerRegistration suggest this will happen,
//! but state that "this will still work".  The problem occurs when what would wake this will run in
//! a lower priority executor.
//!
//! To demonstrate this, we have a low priority executor (thread) that first does a "release" on a
//! semaphore, and then does an "acquire" on a back semaphore.  Then, higher priority tasks do the
//! inverse of this, with multiple of these tasks scheduled.
//!
//! Once the second task tries to wait for the semaphore, the two futures begin to each register
//! their waker, which causes the other one to wake.  The executor will spin doing this, not
//! releasing the CPU to allow the lower priority worker to run.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use defmt::{info, unwrap};
use embassy_executor::{Executor, InterruptExecutor};
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[allow(unused_imports)]
use embassy_sync::channel::Channel;
#[allow(unused_imports)]
use embassy_sync::semaphore::{FairSemaphore, GreedySemaphore, Semaphore};
use embassy_time::{Instant, Timer, TICK_HZ};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// This fails with Channel, and GreedySemaphore. As FairSemaphore has a queue instead of a single
// waker, it does not have the problem.
static SEM: GreedySemaphore<CriticalSectionRawMutex> = GreedySemaphore::new(0);
static BACK_SEM: GreedySemaphore<CriticalSectionRawMutex> = GreedySemaphore::new(0);
/*
static SEM: FairSemaphore<CriticalSectionRawMutex, {NTASKS + 1}> = FairSemaphore::new(0);
static BACK_SEM: FairSemaphore<CriticalSectionRawMutex, {NTASKS + 1}> = FairSemaphore::new(0);
*/
/*
static CHAN: Channel<CriticalSectionRawMutex, (), {NTASKS + 1}> = Channel::new();
static BACK_CHAN: Channel<CriticalSectionRawMutex, (), {NTASKS + 1}> = Channel::new();
*/

const NTASKS: usize = 2;
const ITERS: usize = 10;

#[embassy_executor::task(pool_size = NTASKS)]
async fn med_taker() {
    // Wait a bit before starting, to get past the wait bug, and show that it still fails.
    Timer::after_ticks(1_000_000).await;

    loop {
        info!("Starting taker");
        let start = Instant::now();

        for _ in 0..ITERS {
            let rel = SEM.acquire(1).await.unwrap();
            rel.disarm();
            BACK_SEM.release(1);
            /*
            CHAN.receive().await;
            BACK_CHAN.send(()).await;
            */
        }

        let end = Instant::now();
        let us = end.duration_since(start).as_ticks() * 1_000_000 / TICK_HZ;
        info!("  taker: {} us", us);
    }
}

#[embassy_executor::task]
async fn run_low() {
    // The low priority task releases on SEM, and then acquires BACK_SEM, repeatedly.
    loop {
        info!("Starting giver");
        let start = Instant::now();

        for _ in 0..NTASKS {
            for _ in 0..ITERS {
                SEM.release(1);
                let rel = BACK_SEM.acquire(1).await.unwrap();
                rel.disarm();
                /*
                CHAN.send(()).await;
                BACK_CHAN.receive().await;
                */
            }
        }

        let end = Instant::now();
        let us = end.duration_since(start).as_ticks() * 1_000_000 / TICK_HZ;
        info!("[low] done in {} us", us);
    }
}

static EXECUTOR_MED: InterruptExecutor = InterruptExecutor::new();
static EXECUTOR_LOW: StaticCell<Executor> = StaticCell::new();

#[interrupt]
unsafe fn SWI_IRQ_0() {
    EXECUTOR_MED.on_interrupt()
}

#[entry]
fn main() -> ! {
    let _p = embassy_rp::init(Default::default());

    // Medium-priority executor: SWI_IRQ_0, priority level 3
    interrupt::SWI_IRQ_0.set_priority(Priority::P3);
    let spawner = EXECUTOR_MED.start(interrupt::SWI_IRQ_0);
    for _ in 0..NTASKS {
        spawner.must_spawn(med_taker());
    }

    // Low priority executor: runs in thread mode, using WFE/SEV
    let executor = EXECUTOR_LOW.init(Executor::new());
    executor.run(|spawner| {
        /*
        for _ in 0..NTASKS {
            info!("Spawning taker");
            spawner.must_spawn(med_taker());
            info!("Done spawn");
        }
        */
        unwrap!(spawner.spawn(run_low()));
    });
}
