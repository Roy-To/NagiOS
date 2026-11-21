/* Development-only initramfs probe. It opens /dev/fb0, reports the result on
 * the serial console, then sleeps so the supervisor can observe and stop it.
 * It is not a Nagi service and never reports service readiness.
 */

.section .text
.globl _start
.type _start,@function
_start:
    /* mount("devtmpfs", "/dev", "devtmpfs", MS_NOSUID|MS_NOEXEC, mode=0755) */
    mov $165, %rax
    lea devtmpfs(%rip), %rdi
    lea dev_path(%rip), %rsi
    lea devtmpfs(%rip), %rdx
    mov $10, %r10
    lea mount_options(%rip), %r8
    syscall
    test %rax, %rax
    jns .Lopen_fb0
    cmp $-16, %rax
    je .Lopen_fb0
    mov $1, %rax
    mov $1, %rdi
    lea mount_failure(%rip), %rsi
    mov $mount_failure_len, %rdx
    syscall
    jmp .Lsleep

.Lopen_fb0:
    /* openat(AT_FDCWD, "/dev/fb0", O_RDONLY, 0) */
    mov $257, %rax
    mov $-100, %rdi
    lea fb0_path(%rip), %rsi
    xor %rdx, %rdx
    xor %r10, %r10
    syscall
    test %rax, %rax
    js .Lopen_failed

    mov %rax, %rdi
    mov $3, %rax
    syscall
    mov $1, %rax
    mov $1, %rdi
    lea success(%rip), %rsi
    mov $success_len, %rdx
    syscall
    jmp .Lsleep

.Lopen_failed:
    cmp $-2, %rax
    je .Lmissing
    cmp $-13, %rax
    je .Laccess_denied
    cmp $-19, %rax
    je .Ldevice_missing
    mov $1, %rax
    mov $1, %rdi
    lea other_failure(%rip), %rsi
    mov $other_failure_len, %rdx
    syscall
    jmp .Lsleep

.Lmissing:
    mov $1, %rax
    mov $1, %rdi
    lea missing(%rip), %rsi
    mov $missing_len, %rdx
    syscall
    jmp .Lsleep

.Laccess_denied:
    mov $1, %rax
    mov $1, %rdi
    lea access_denied(%rip), %rsi
    mov $access_denied_len, %rdx
    syscall
    jmp .Lsleep

.Ldevice_missing:
    mov $1, %rax
    mov $1, %rdi
    lea device_missing(%rip), %rsi
    mov $device_missing_len, %rdx
    syscall

.Lsleep:
    /* nanosleep({60, 0}, NULL), repeated forever. */
    mov $35, %rax
    lea delay(%rip), %rdi
    xor %rsi, %rsi
    syscall
    jmp .Lsleep

.section .rodata
fb0_path:
    .asciz "/dev/fb0"
success:
    .ascii "NAGI_FBDEV_PROBE=PASS\n"
.set success_len, .-success
missing:
    .ascii "NAGI_FBDEV_PROBE=MISSING:ENOENT\n"
.set missing_len, .-missing
access_denied:
    .ascii "NAGI_FBDEV_PROBE=OPEN_FAILED:EACCES\n"
.set access_denied_len, .-access_denied
device_missing:
    .ascii "NAGI_FBDEV_PROBE=OPEN_FAILED:ENODEV\n"
.set device_missing_len, .-device_missing
other_failure:
    .ascii "NAGI_FBDEV_PROBE=OPEN_FAILED:OTHER\n"
.set other_failure_len, .-other_failure
mount_failure:
    .ascii "NAGI_FBDEV_PROBE=MOUNT_FAILED\n"
.set mount_failure_len, .-mount_failure
devtmpfs:
    .asciz "devtmpfs"
dev_path:
    .asciz "/dev"
mount_options:
    .asciz "mode=0755"

.section .data
.align 8
delay:
    .quad 60, 0
