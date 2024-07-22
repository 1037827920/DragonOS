use core::sync::atomic::{compiler_fence, Ordering};

use log::debug;
use system_error::SystemError;
use x86::dtables::DescriptorTablePointer;

use crate::{
    arch::{interrupt::trap::arch_trap_init, process::table::TSSManager},
    driver::clocksource::acpi_pm::init_acpi_pm_clocksource,
    init::init::start_kernel,
    mm::{MemoryManagementArch, PhysAddr},
};

use super::{
    driver::{
        hpet::{hpet_init, hpet_instance},
        tsc::TSCManager,
    },
    MMArch,
};

#[derive(Debug)]
pub struct ArchBootParams {}

impl ArchBootParams {
    pub const DEFAULT: Self = ArchBootParams {};
}

extern "C" {
    static mut GDT_Table: [usize; 0usize];
    static mut IDT_Table: [usize; 0usize];
    fn head_stack_start();

    fn multiboot2_init(mb2_info: u64, mb2_magic: u32) -> bool;
}

/// 内核的主入口点
#[no_mangle]
unsafe extern "C" fn kernel_main(
    mb2_info: u64, // 多引导信息
    mb2_magic: u64, // 魔法数
    bsp_gdt_size: u64, // GDT大小
    bsp_idt_size: u64, // IDT大小
) -> ! {
    let mut gdtp = DescriptorTablePointer::<usize>::default();
    // 将GDT和IDT的物理地址转换为虚拟地址
    let gdt_vaddr =
        MMArch::phys_2_virt(PhysAddr::new(&GDT_Table as *const usize as usize)).unwrap();
    let idt_vaddr =
        MMArch::phys_2_virt(PhysAddr::new(&IDT_Table as *const usize as usize)).unwrap();
    // 设置GDT和IDT的基址和限制
    gdtp.base = gdt_vaddr.data() as *const usize;
    gdtp.limit = bsp_gdt_size as u16 - 1;

    let idtp = DescriptorTablePointer::<usize> {
        base: idt_vaddr.data() as *const usize,
        limit: bsp_idt_size as u16 - 1,
    };

    // 加载GDT和IDT
    x86::dtables::lgdt(&gdtp);
    x86::dtables::lidt(&idtp);

    // 使用compiler_fence确保内存操作的顺序
    compiler_fence(Ordering::SeqCst);
    // 初始化多引导信息
    multiboot2_init(mb2_info, (mb2_magic & 0xFFFF_FFFF) as u32);
    compiler_fence(Ordering::SeqCst);

    // 启动内核
    start_kernel();
}

/// 在内存管理初始化之前的架构相关的早期初始化
#[inline(never)]
pub fn early_setup_arch() -> Result<(), SystemError> {
    let stack_start = unsafe { *(head_stack_start as *const u64) } as usize;
    debug!("head_stack_start={:#x}\n", stack_start);
    unsafe {
        let gdt_vaddr =
            MMArch::phys_2_virt(PhysAddr::new(&GDT_Table as *const usize as usize)).unwrap();
        let idt_vaddr =
            MMArch::phys_2_virt(PhysAddr::new(&IDT_Table as *const usize as usize)).unwrap();

        debug!("GDT_Table={:?}, IDT_Table={:?}\n", gdt_vaddr, idt_vaddr);
    }

    // 设置当前核心的任务状态段(TSS)，这是处理中断和任务切换时必须正确设置的另一个关键数据结构
    set_current_core_tss(stack_start, 0);
    // 加载任务寄存器(TR)，这是启动任务切换机制的必要步骤
    unsafe { TSSManager::load_tr() };
    // 初始化trap和中断处理机制，确保系统能够响应硬件中断和异常
    arch_trap_init().expect("arch_trap_init failed");

    return Ok(());
}

/// 架构相关的初始化
#[inline(never)]
pub fn setup_arch() -> Result<(), SystemError> {
    return Ok(());
}

/// 架构相关的初始化（在IDLE的最后一个阶段）
#[inline(never)]
pub fn setup_arch_post() -> Result<(), SystemError> {
    let ret = hpet_init();
    if ret.is_ok() {
        hpet_instance().hpet_enable().expect("hpet enable failed");
    } else {
        init_acpi_pm_clocksource().expect("acpi_pm_timer inits failed");
    }
    TSCManager::init().expect("tsc init failed");

    return Ok(());
}

fn set_current_core_tss(stack_start: usize, ist0: usize) {
    let current_tss = unsafe { TSSManager::current_tss() };
    debug!(
        "set_current_core_tss: stack_start={:#x}, ist0={:#x}\n",
        stack_start, ist0
    );
    current_tss.set_rsp(x86::Ring::Ring0, stack_start as u64);
    current_tss.set_ist(0, ist0 as u64);
}
