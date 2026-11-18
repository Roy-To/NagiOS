.section .text
.global _start
.type _start,@function

# Raw syscalls return negative errno values in %rax. The harness must first
# prove the same-credential no-Landlock controls succeed; for rename it must
# also prove both endpoint directories share st_dev before EXDEV is accepted.
.macro EXPECT_EACCES bit
    cmp $-13, %rax
    je .Lexpected_eacces_\@
    bts $\bit, %r14d
.Lexpected_eacces_\@:
.endm

# A cross-root rename may be denied with EACCES or EXDEV by Landlock's REFER
# rules. ENOENT, ENOSYS, EINVAL, and all other results remain failures.
.macro EXPECT_RENAME_DENIED bit
    cmp $-13, %rax
    je .Lexpected_rename_\@
    cmp $-18, %rax
    je .Lexpected_rename_\@
    bts $\bit, %r14d
.Lexpected_rename_\@:
.endm

_start:
    mov (%rsp), %r12
    lea 8(%rsp), %r13
    xor %r14d, %r14d
    cmp $10, %r12
    jl .Lbad_args

    # Another App: raw read, write, and create must fail.
    mov $257, %rax
    mov $-100, %rdi
    mov 8(%r13), %rsi
    xor %edx, %edx
    xor %r10d, %r10d
    syscall
    cmp $-13, %rax
    je .Lforeign_read_denied
    test %rax, %rax
    js .Lforeign_read_error
    bts $0, %r14d
    mov %rax, %rdi
    mov $3, %rax
    syscall
    jmp .Lforeign_read_done
.Lforeign_read_error:
    EXPECT_EACCES 0
.Lforeign_read_done:
.Lforeign_read_denied:
    mov $257, %rax
    mov $-100, %rdi
    mov 8(%r13), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    cmp $-13, %rax
    je .Lforeign_write_done
    test %rax, %rax
    js .Lforeign_write_error
    bts $1, %r14d
    mov %rax, %rdi
    mov $3, %rax
    syscall
    jmp .Lforeign_write_done
.Lforeign_write_error:
    EXPECT_EACCES 1
.Lforeign_write_done:
.Lforeign_write_denied:
    mov $257, %rax
    mov $-100, %rdi
    mov 16(%r13), %rsi
    mov $577, %edx
    mov $0600, %r10d
    syscall
    cmp $-13, %rax
    je .Lforeign_create_done
    test %rax, %rax
    js .Lforeign_create_error
    bts $2, %r14d
    mov %rax, %rdi
    mov $3, %rax
    syscall
    jmp .Lforeign_create_done
.Lforeign_create_error:
    EXPECT_EACCES 2
.Lforeign_create_done:
.Lforeign_create_denied:

    # Cross-root rename must fail in both directions.
    mov $316, %rax
    mov $-100, %rdi
    mov 24(%r13), %rsi
    mov $-100, %rdx
    mov 32(%r13), %r10
    xor %r8d, %r8d
    syscall
    EXPECT_RENAME_DENIED 3
.Lforeign_to_allowed_denied:
    mov $316, %rax
    mov $-100, %rdi
    mov 40(%r13), %rsi
    mov $-100, %rdx
    mov 48(%r13), %r10
    xor %r8d, %r8d
    syscall
    EXPECT_RENAME_DENIED 4
.Lallowed_to_foreign_denied:

    # A different Profile root must not be readable or writable.
    mov $257, %rax
    mov $-100, %rdi
    mov 56(%r13), %rsi
    xor %edx, %edx
    xor %r10d, %r10d
    syscall
    cmp $-13, %rax
    je .Lprofile_read_done
    test %rax, %rax
    js .Lprofile_read_error
    bts $5, %r14d
    mov %rax, %rdi
    mov $3, %rax
    syscall
    jmp .Lprofile_read_done
.Lprofile_read_error:
    EXPECT_EACCES 5
.Lprofile_read_done:
.Lprofile_read_denied:
    mov $257, %rax
    mov $-100, %rdi
    mov 56(%r13), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    cmp $-13, %rax
    je .Lprofile_write_done
    test %rax, %rax
    js .Lprofile_write_error
    bts $6, %r14d
    mov %rax, %rdi
    mov $3, %rax
    syscall
    jmp .Lprofile_write_done
.Lprofile_write_error:
    EXPECT_EACCES 6
.Lprofile_write_done:
.Lprofile_write_denied:

    # Existing LinuxProcessOs marks every inherited fd >= 3 CLOEXEC.
    # Accept only one canonical positive decimal fd in the supported int range.
    xor %ebx, %ebx
    mov 64(%r13), %rsi
    movzbl (%rsi), %eax
    test %al, %al
    je .Lbad_args
    cmp $'0', %al
    jne .Lparse_fd
    cmpb $0, 1(%rsi)
    jne .Lbad_args
.Lparse_fd:
    movzbl (%rsi), %eax
    test %al, %al
    je .Lcheck_fd
    sub $'0', %al
    cmp $9, %al
    ja .Lbad_args
    cmp $214748364, %ebx
    ja .Lbad_args
    jne .Lfd_accumulate
    cmp $7, %eax
    ja .Lbad_args
.Lfd_accumulate:
    imul $10, %ebx, %ebx
    movzbl %al, %eax
    add %eax, %ebx
    inc %rsi
    jmp .Lparse_fd
.Lcheck_fd:
    cmp $3, %ebx
    jb .Lbad_args
    mov $72, %rax
    mov %rbx, %rdi
    mov $1, %rsi
    xor %edx, %edx
    syscall
    cmp $-9, %rax
    je .Lheld_fd_closed
    bts $7, %r14d
.Lheld_fd_closed:

    # A Notes file in the single authorized data root remains writable.
    mov $257, %rax
    mov $-100, %rdi
    mov 72(%r13), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    test %rax, %rax
    js .Lallowed_write_failed
    mov %rax, %rdi
    lea .Lbyte(%rip), %rsi
    mov $1, %rdx
    mov $1, %rax
    syscall
    cmp $1, %rax
    je .Ldone
.Lallowed_write_failed:
    bts $8, %r14d
.Ldone:
    xor %edi, %edi
    test %r14d, %r14d
    setne %dil
    mov $231, %rax
    syscall
.Lbad_args:
    mov $2, %edi
    mov $231, %rax
    syscall
    ud2
.size _start, .-_start

.section .rodata
.Lbyte:
    .byte 'x'
.section .note.GNU-stack,"",@progbits
