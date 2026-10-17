# All generated binaries and the staged ISO filesystem live under build/.
BUILD_DIR := build/artifacts
ISO_ROOT := $(BUILD_DIR)/iso
KERNEL := $(BUILD_DIR)/kernel.elf
RUST_LIBRARY := $(BUILD_DIR)/libhorizon_kernel.a
BOOT_OBJECT := $(BUILD_DIR)/boot.o
RUNTIME_OBJECT := $(BUILD_DIR)/runtime.o
PE_FIXTURE := $(BUILD_DIR)/hello-fixture.exe
INITRD := $(BUILD_DIR)/initrd.img
ISO := $(BUILD_DIR)/horizon-os.iso

CC ?= gcc
LD ?= ld
RUSTC ?= rustc
GRUB_MKRESCUE ?= grub-mkrescue
QEMU ?= qemu-system-i386
QEMU_DISPLAY ?= gtk

# Building the ISO is deliberately separate from launching the emulator.
.PHONY: all build kernel iso run test test-pe test-clock test-keyboard test-mouse test-heap test-filesystem clean

all: build

build: iso

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Assemble the Multiboot entry and its interrupt/context-switch stubs.
$(BOOT_OBJECT): src/boot.s | $(BUILD_DIR)
	$(CC) -m32 -c $< -o $@

# Provide the small freestanding libc/compiler-runtime symbols used by Rust.
$(RUNTIME_OBJECT): src/runtime.c | $(BUILD_DIR)
	$(CC) -m32 -ffreestanding -fno-builtin -fno-stack-protector -c $< -o $@

# Compile the no_std Rust kernel as a static library for the i686 target.
$(RUST_LIBRARY): src/lib.rs src/memory.rs src/heap.rs src/pe.rs src/graphics.rs src/mouse.rs src/window_manager.rs src/clock.rs src/font8x8.rs src/keyboard.rs src/scheduler.rs src/filesystem.rs | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --target i686-unknown-linux-gnu \
		--crate-name horizon_kernel --crate-type staticlib \
		-C panic=abort -C opt-level=2 -C relocation-model=static \
		$< -o $@

# Link at the Multiboot-required physical load address defined in linker.ld.
$(KERNEL): $(BOOT_OBJECT) $(RUNTIME_OBJECT) $(RUST_LIBRARY) linker.ld
	$(LD) -m elf_i386 -T linker.ld -nostdlib -o $@ $(BOOT_OBJECT) $(RUNTIME_OBJECT) $(RUST_LIBRARY)

kernel: $(KERNEL)

$(ISO_ROOT)/boot/kernel.elf: $(KERNEL)
	mkdir -p $(dir $@)
	cp $< $@

$(ISO_ROOT)/boot/grub/grub.cfg: iso/boot/grub/grub.cfg
	mkdir -p $(dir $@)
	cp $< $@

$(PE_FIXTURE): tools/gen_pe_fixture.py | $(BUILD_DIR)
	python3 $< $@

$(INITRD): tools/gen_initrd.py | $(BUILD_DIR)
	python3 $< $@

$(ISO_ROOT)/boot/hello-fixture.exe: $(PE_FIXTURE)
	mkdir -p $(dir $@)
	cp $< $@

$(ISO_ROOT)/boot/initrd.img: $(INITRD)
	mkdir -p $(dir $@)
	cp $< $@

# Stage boot files and both Multiboot modules into a bootable GRUB ISO.
$(ISO): $(ISO_ROOT)/boot/kernel.elf $(ISO_ROOT)/boot/grub/grub.cfg $(ISO_ROOT)/boot/hello-fixture.exe $(ISO_ROOT)/boot/initrd.img
	$(GRUB_MKRESCUE) -o $@ $(ISO_ROOT)

iso: $(ISO)

# Run the already-built image only when explicitly requested.
run: $(ISO)
	$(QEMU) -display $(QEMU_DISPLAY) -vga std -cdrom $(ISO) -m 128M

test-pe: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/pe.rs -o $(BUILD_DIR)/pe-tests
	$(BUILD_DIR)/pe-tests

test-clock: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/clock.rs -o $(BUILD_DIR)/clock-tests
	$(BUILD_DIR)/clock-tests

test-keyboard: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/keyboard.rs -o $(BUILD_DIR)/keyboard-tests
	$(BUILD_DIR)/keyboard-tests

test-mouse: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/mouse.rs -o $(BUILD_DIR)/mouse-tests
	$(BUILD_DIR)/mouse-tests

test-heap: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/heap.rs -o $(BUILD_DIR)/heap-tests
	$(BUILD_DIR)/heap-tests

test-filesystem: | $(BUILD_DIR)
	$(RUSTC) --edition=2021 --test src/filesystem.rs -o $(BUILD_DIR)/filesystem-tests
	$(BUILD_DIR)/filesystem-tests

test: test-pe test-clock test-keyboard test-mouse test-heap test-filesystem

clean:
	rm -rf build
