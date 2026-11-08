; primes below 128 by trial division
; r0 = n (candidate), r1 = d (divisor), r2 = m (running remainder), r3 = scratch
        ldi r0, 2          ; n = 2
        ldi r1, 2          ; d = 2
inner:
        sub r0, r1, r3     ; r3 = n - d
        jz r3, prime       ; d reached n with no divisor: n is prime
        or r0, r0, r2      ; m = n   (there is no mov, so or-with-itself)
shave:
        sub r2, r1, r2     ; m = m - d
        jz r2, composite   ; m hit exactly zero: d divides n
        ldi r3, 128
        and r2, r3, r3     ; r3 = m & 0x80, the sign bit
        jz r3, shave       ; sign clear: m still >= 0, keep subtracting
        ldi r3, 1          ; m went negative: d does not divide n
        add r1, r3, r1     ; d = d + 1
        jmp inner
prime:
        out r0
composite:
        ldi r1, 2          ; d = 2 again
        ldi r3, 1
        add r0, r3, r0     ; n = n + 1
        ldi r3, 128
        sub r0, r3, r3     ; zero once n reaches 128
        jz r3, done
        jmp inner
done:
        hlt