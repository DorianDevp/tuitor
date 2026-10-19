/* Totals a few orders; gold and silver customers get a discount. Prices are in cents. */

#include <stdio.h>

#define MAX_LINES 4

enum tier { REGULAR, SILVER, GOLD };

struct address {
    const char *city;
    const char *country;
};

struct customer {
    const char *name;
    enum tier tier;
    struct address address;
};

struct line {
    const char *sku;
    int qty;
    long price;
};

struct order {
    int id;
    struct customer customer;
    struct line lines[MAX_LINES];
    int line_count;
};

static long line_total(const struct line *line)
{
    return line->qty * line->price;
}

static long discount(const struct customer *customer, long amount)
{
    static const int percent[] = {[REGULAR] = 0, [SILVER] = 5, [GOLD] = 10};
    return amount * percent[customer->tier] / 100;
}

static long order_total(const struct order *order)
{
    long total = 0;
    for (int i = 0; i < order->line_count; i++) {
        total += line_total(&order->lines[i]);
    }
    return total - discount(&order->customer, total);
}

int main(void)
{
    const struct order orders[] = {
        {1001, {"Ada", GOLD, {"London", "UK"}},
         {{"KB-01", 1, 8900}, {"MS-02", 2, 2450}, {"CB-03", 3, 990}}, 3},
        {1002, {"Grace", SILVER, {"Arlington", "US"}},
         {{"MN-27", 1, 21900}, {"HS-01", 1, 5900}}, 2},
        {1003, {"Linus", REGULAR, {"Helsinki", "FI"}},
         {{"SSD-1T", 2, 7450}}, 1},
    };
    const int count = sizeof orders / sizeof orders[0];

    long revenue = 0;
    for (int i = 0; i < count; i++) {
        const struct order *order = &orders[i];
        long total = order_total(order);
        revenue += total;
        const struct customer *customer = &order->customer;
        printf("#%d %s (%s, %s):", order->id, customer->name, customer->address.city, customer->address.country);
        for (int j = 0; j < order->line_count; j++) {
            printf(" %s", order->lines[j].sku);
        }
        printf(" = %ld.%02ld\n", total / 100, total % 100);
    }
    printf("revenue %ld.%02ld\n", revenue / 100, revenue % 100);
    return 0;
}
