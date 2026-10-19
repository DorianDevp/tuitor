# -O0 keeps every variable visible in the debugger.
CFLAGS = -g -O0 -Wall -Wextra

orders: orders.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f orders

.PHONY: clean
