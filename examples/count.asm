ldi r0, 0        ; counter
ldi r1, 1        ; the number 1
ldi r2, 10       ; limit
loop:
out r0           ; send the counter to the output port
sub r0, r2, r3   ; r3 = r0 - r2  (zero when we've reached the limit)
jz r3, done
add r0, r1, r0   ; counter = counter + 1
jmp loop
done:
hlt