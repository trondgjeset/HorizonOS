# Horizon OS 🌅
### A Clean-Slate, Open-Source Windows Alternative Built in Rust & Assembly

**Horizon OS** is an ambitious, independent, open-source operating system project designed from scratch to be a mainstream alternative to Microsoft Windows. It aims to deliver a platform that runs standard Windows executables (`.exe`) natively, without the weight of Linux or a hypervisor, while completely eliminating the corporate bloatware, forced AI tracking, and security vulnerabilities that plague modern computing.

---

## 📢 A Note from the Founder & Project Lead
Hi there! I want to be 100% transparent upfront: **I am not a professional systems programmer or software engineer.** 

I am a tech enthusiast who became deeply frustrated with the current direction of mainstream operating systems. I wanted an OS that was simple, intuitive, and safe, but found that existing alternatives were either decades-long alpha projects or fragmented into thousands of confusing Linux distributions.

To break the stalemate, I utilized advanced AI architecture tools to layout the blueprint, establish the compilation toolchains, and code the initial working proof-of-concept framework you see in this repository today. 

**My Role:** I am but a man with a vision. I claim credit only for the core architectural concept, the product strategy, and the long-term vision. I am looking for skilled systems programmers, kernel developers, and UI designers who share this frustration to step in, take over the code, tear down the AI scaffolding where needed, and build this into the global platform it deserves to be.

---

## 🎯 The Core Manifesto

Horizon OS is built on four uncompromised structural mandates to bypass both the security crises of Windows and the fragmentation traps of Linux:

1. **Independent Monolithic Architecture:** Horizon OS is *not* Linux. It is a clean-slate monolithic kernel written in **Rust** (for compile-time memory safety) with **Assembly/C** low-level hardware initialization. It executes Windows Portable Executable (`.exe`) binaries natively by intercepting Win32 system calls at the kernel layer.
2. **Immunized by Design (Intent-Driven Sandbox):** Horizon OS completely eliminates the need for traditional, reactive antivirus software. Applications run isolated in Ring 3 memory "silos" by default. Temporary access to files or hardware is granted seamlessly and invisibly based on physical user intent (e.g., clicking a native File Open dialogue), trapping malware or ransomware automatically.
3. **Terminal-Free & Self-Healing:** To protect the everyday user, Horizon OS utilizes an immutable (read-only) core system image with background atomic updates. If an update or a driver fails, the kernel automatically rolls back to the last working state upon reboot. The user never encounters a command line or a blue screen.
4. **The "One True Platform" Standard:** To eliminate choice fatigue and software distribution fragmentation, Horizon OS mandates a single, highly performant graphical compositor shell. Visual adaptability is treated as a first-class citizen; users can shift layouts dynamically (Classic Windows layout, Centered modern layout, compact layout) via text-based configurations, but software developers only ever have to target *one* unified set of system APIs.

---

## 🛠 Current Project Status (Phase 1 Proof-of-Concept)
The AI-assisted initialization phase has successfully deployed a functional, bootable kernel environment. Currently, Horizon OS can be built via Linux Mint and run safely inside the QEMU emulator.

**What is working right now:**
* **Multiboot Entry:** Boots cleanly via GRUB into a 1024x768 32-bit Linear Framebuffer.
* **Preemptive Multi-tasking:** A core scheduler governing Ring 0 (kernel space) and Ring 3 (isolated user space) tasks via PIT timer ticks (10ms time slices).
* **Interactive GUI Shell:** A double-buffered rendering pipeline tracking live mouse cursor coordinates, window objects, dragging mechanics, a functional Start Menu panel overlay, and embedded bitmap typography.
* **Dynamic Memory Allocator:** A thread-safe global kernel Heap Allocator unlocking native Rust dynamic collections (`Vec`, `String`).
* **Virtual Storage Drive:** A read-only initrd-backed `C:\` drive with a bounded, volatile heap overlay for native shell writes.

### Win32 graphics prototype

The desktop renderer uses a centralized charcoal-slate `Theme` palette with
off-white text and muted steel title bars. Ice Blue (`#4A90E2`) is reserved for
active indicators such as the Start button and focused editor field; idle
controls and taskbar tabs use subtle neutral borders. Window frames, launcher
rows, tabs, and editor controls use integer-distance anti-aliased rounded
corners; floating windows and the Start Menu receive layered alpha-blended
shadows. The Start Menu and title bars are composited over the double buffer
using source-over RGBA pixel blending; the compositor then flushes the
completed back buffer to the linear framebuffer. Customize the shared palette
and pixel math in `src/graphics.rs`.

To preview the theme, build and run the emulator with `make run`.

The desktop also loads a generated 24-bit BMP from
`C:\Windows\System\wallpaper.bmp` in the initrd and scales it to the framebuffer
before composing windows. The BMP decoder accepts bounded, uncompressed 24-bit
and 32-bit `BI_RGB` files and rejects truncated or unsupported images. Click the
editor's rounded input field to focus it, type text, and use Backspace to edit;
the caret blinks while the field has focus. Keyboard IRQ code only queues
translated ASCII—the desktop task mutates the allocated string outside interrupt
context.

The left desktop column contains evenly spaced This PC, User Folder, and
Recycle Bin shortcuts with centered labels and geometric theme-colored
silhouettes. Select an icon with one click to show a translucent Ice Blue
boundary, then open it with a second click on the same icon within 450 ms: This
PC lists the mounted `C:\` volume and an unmounted `D:\` placeholder, while the
other shortcuts open the user's Documents directory and the sandbox
`C:\System\Trash` directory. The icons are composited above the wallpaper and
behind application windows.

The Start Menu's **Run Win32 Demo** option opens a normal Horizon window and
draws through the experimental `TextOutA` and `FillRect` GDI-shaped kernel
hooks. To try it, build and start the emulator with `make run`, open Start, and
select **Run Win32 Demo**.

The **Launch External App** option reads the data-only PE32 fixture at
`C:\Apps\TEST.EXE` from the read-only RAM disk. It validates the headers,
copies section bytes into an RVA-layout staging buffer, and classifies a small
set of GDI imports. The primary window reports the image base, entry RVA, size,
and sections; it does **not** execute the fixture.

The desktop starts with no application window open. Select **System Utility**
from the Start Menu to allocate and open the editor on demand. It includes the
shared-page calculator task counter, a small text input field, and a **Save
File** button. The isolated Ring 3 calculator context is prepared during boot
but remains blocked until System Utility opens successfully. A click writes
the entered bytes to `C:\Users\Default\Documents\notes.txt` in a bounded, heap-backed writable
overlay; File Explorer lists the result and reads its latest contents. This
overlay is volatile and is lost at reboot. The
tar initrd remains unchanged; durable writes require a future journaled block
storage backend.

The `CreateFileA` and `WriteFile` C-ABI names currently fail closed and report
`ERROR_ACCESS_DENIED` because no Ring 3 syscall dispatcher or safe guest-memory
copy exists. The native Save button uses a separate fixed-path kernel capability;
it is not proof of process-bound Win32 authorization or a guest file API.

This is a native in-kernel rendering smoke test, not Windows application
execution: there is no Win32 import dispatcher, guest-pointer marshalling,
process-owned GDI context, dynamic per-process page-directory allocator,
relocation/IAT patching, or dynamic scheduler registration. Import matches are
metadata only and do not contain kernel function pointers. The Direct3D/Vulkan
placeholders fail closed until those isolation and ABI facilities exist. Run
`make test-graphics`, `make test-win32`, `make test-win32-loader`, and
`make test-win32-file-api` to exercise rounded-edge/color math, hook coordinate
conversion, PE staging, import classification, and the fail-closed file
API/native save capability.

---

## 🤝 How to Contribute (Open-Source Call to Arms)
The foundation is laid, heavily commented, and fully bootable. We are officially opening the gates for skilled developers to optimize, refactor, and scale this project. 

Immediate areas looking for expert contributions:
* **The PE Loader Expansion:** Moving our PE32 fixture parser into a full-scale memory mapping routine for external binary execution.
* **The Win32 Interceptor Lookup Table:** Mapping the early interrupt/syscall gates to route basic Windows NT application requests.
* **Driver Wrapper Subsystem:** Prototyping a kernel-level framework to wrap or bridge open-source graphics and hardware drivers.

If you are an engineer who loves doing things "the hard way" because it results in something fundamentally better, pull up a chair, review the commented code, and let's build a better horizon for personal computing together.

---

## ⚖️ License
Horizon OS is proudly licensed under the **GNU General Public License v3 (G GPLv3)**. See the `LICENSE` file for full details. Copying code to create closed, proprietary versions of this system is strictly prohibited. Horizon OS will remain free and open to humanity forever.
