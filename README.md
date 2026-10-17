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
* **Virtual Storage Drive:** A simulated read-only `C:\` drive running via an integrated RAM disk layout.

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
