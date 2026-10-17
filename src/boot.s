.section .multiboot,"a"
.align 4
/* Multiboot v1 header: request aligned modules, memory information, and video mode. */
.long 0x1BADB002
.long 0x00000007
.long -(0x1BADB002 + 0x00000007)
/* Video request: GRUB should provide a 1024x768x32 linear framebuffer. */
.long 0
.long 1024
.long 768
.long 32

.section .text
.code32
.global _start
.type _start, @function
_start:
    cli
    /* Preserve Multiboot EAX magic; EBX remains the info-structure argument. */
    mov %eax, %esi
    /* Clear CR0.EM and set CR0.MP before enabling floating-point state support. */
    mov %cr0, %eax
    and $-5, %eax
    or $2, %eax
    mov %eax, %cr0
    /* Enable CR4.OSFXSR and CR4.OSXMMEXCPT for FXSAVE/FXRSTOR context switching. */
    mov %cr4, %eax
    or $0x600, %eax
    mov %eax, %cr4
    /* Switch to our aligned 64 KiB bootstrap stack and call Rust as cdecl:
       rust_kernel_main(boot_magic, multiboot_info_address). */
    mov %esi, %eax
    mov $stack_top, %esp
    and $-16, %esp
    xor %ebp, %ebp
    sub $8, %esp
    push %ebx
    push %eax
    call rust_kernel_main
    add $16, %esp

halt:
    cli
    hlt
    jmp halt
.size _start, . - _start

.global load_gdt
.type load_gdt, @function
load_gdt:
    mov 4(%esp), %eax
    lgdt (%eax)
    /* Reload CS with the kernel code selector; then reload all data selectors.
       User-mode transitions later use ring-3 selectors and an IRET frame. */
    ljmp $0x08, $gdt_loaded
gdt_loaded:
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss
    ret
.size load_gdt, . - load_gdt

.global load_idt
.type load_idt, @function
load_idt:
    mov 4(%esp), %eax
    /* Install the IDT descriptor prepared by Rust before interrupts are enabled. */
    lidt (%eax)
    ret
.size load_idt, . - load_idt

.global irq1_stub
.type irq1_stub, @function
irq1_stub:
    pushal
    cld
    mov %esp, %ebp
    and $-16, %esp
    call rust_irq1_handler
    mov %ebp, %esp
    popal
    iret
.size irq1_stub, . - irq1_stub

.global irq0_stub
.type irq0_stub, @function
irq0_stub:
    /* Preserve segment and general registers, then save the outgoing FPU image.
       Rust returns the next thread's saved stack pointer in EAX. */
    push %ds
    push %es
    push %fs
    push %gs
    pushal
    cld
    mov CURRENT_FPU_STATE, %edx
    fxsave (%edx)
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %esp, %ebp
    and $-16, %esp
    sub $12, %esp
    push %ebp
    call rust_irq0_handler
    /* Restore the selected thread's FPU image and stack; IRET completes the
       context switch (including a ring transition when CS is a user selector). */
    mov CURRENT_FPU_STATE, %edx
    fxrstor (%edx)
    mov %eax, %esp
    popal
    pop %gs
    pop %fs
    pop %es
    pop %ds
    iret
.size irq0_stub, . - irq0_stub

.global scheduler_start
.type scheduler_start, @function
scheduler_start:
    cli
    mov 4(%esp), %eax
    mov 8(%esp), %edx
    mov %eax, %cr3
    /* Restore the first task with the same frame shape used by irq0_stub. */
    mov CURRENT_FPU_STATE, %eax
    fxrstor (%eax)
    mov %edx, %esp
    popal
    pop %gs
    pop %fs
    pop %es
    pop %ds
    /* With a ring-3 CS selector, IRET also consumes the prepared user SS:ESP.
       Later ring-3 interrupts obtain their ring-0 SS:ESP from the loaded TSS. */
    iret
.size scheduler_start, . - scheduler_start

.global load_tss
.type load_tss, @function
load_tss:
    mov 4(%esp), %eax
    ltr %ax
    ret
.size load_tss, . - load_tss

.global irq12_stub
.type irq12_stub, @function
irq12_stub:
    pushal
    cld
    mov %esp, %ebp
    and $-16, %esp
    call rust_irq12_handler
    mov %ebp, %esp
    popal
    iret
.size irq12_stub, . - irq12_stub

.global unexpected_interrupt_stub
.type unexpected_interrupt_stub, @function
unexpected_interrupt_stub:
    cli
    and $-16, %esp
    call rust_unexpected_interrupt_handler
    ud2
.size unexpected_interrupt_stub, . - unexpected_interrupt_stub

.section .bss
.align 16
stack_bottom:
    .skip 65536
stack_top:

.section .usertext,"ax"
.align 4096
.global user_calculation_entry
.global user_calculation_end
.type user_calculation_entry, @function
user_calculation_entry:
    /* Demonstration ring-3 workload: update its user-mapped shared counter. */
    mov $0x00500000, %edi
1:
    lock incl (%edi)
    pause
    jmp 1b
user_calculation_end:
.size user_calculation_entry, . - user_calculation_entry

.section .note.GNU-stack,"",@progbits
