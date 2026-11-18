/*
 * Freestanding x86-64 Linux startup for the development login child.
 * The C3 process owner supplies exactly one inherited Unix control endpoint.
 * Keep it live until the owner closes it. The input transport must be bound
 * separately before this child can read or submit credentials.
 */
.section .text
.global _start
_start:
    /* getpeername(3, &address, &length) confirms a connected endpoint. */
    sub $144, %rsp
    movl $128, 128(%rsp)
    mov $52, %eax
    mov $3, %edi
    mov %rsp, %rsi
    lea 128(%rsp), %rdx
    syscall
    test %rax, %rax
    js invalid_endpoint
    cmpl $2, 128(%rsp)
    jb invalid_endpoint
    cmpw $1, (%rsp)
    jne invalid_endpoint

    /* poll only for peer closure. This bootstrap consumes no application data. */
    movl $3, (%rsp)
    movl $0, 4(%rsp)
wait_for_owner:
    mov $7, %eax
    mov %rsp, %rdi
    mov $1, %esi
    mov $-1, %edx
    syscall
    cmp $-4, %rax
    je wait_for_owner
    test %rax, %rax
    jle invalid_endpoint
    testw $0x28, 6(%rsp)
    jnz invalid_endpoint
    testw $0x10, 6(%rsp)
    jz wait_for_owner
    xor %edi, %edi
    jmp finish
invalid_endpoint:
    mov $69, %edi
finish:
    mov $60, %eax
    syscall
    ud2

.section .note.GNU-stack,"",@progbits
