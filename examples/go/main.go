// Totals a few orders; gold and silver customers get a discount. Prices are in cents.
package main

import (
	"fmt"
	"strings"
)

type Tier int

const (
	Regular Tier = iota
	Silver
	Gold
)

type Address struct {
	City    string
	Country string
}

type Customer struct {
	Name    string
	Tier    Tier
	Address Address
}

type Line struct {
	SKU   string
	Qty   int
	Price int
}

type Order struct {
	ID       int
	Customer Customer
	Lines    []Line
}

func lineTotal(line Line) int {
	return line.Qty * line.Price
}

func discount(customer Customer, amount int) int {
	percent := 0
	switch customer.Tier {
	case Silver:
		percent = 5
	case Gold:
		percent = 10
	}
	return amount * percent / 100
}

func orderTotal(order Order) int {
	total := 0
	for _, line := range order.Lines {
		total += lineTotal(line)
	}
	return total - discount(order.Customer, total)
}

func money(cents int) string {
	return fmt.Sprintf("%d.%02d", cents/100, cents%100)
}

func main() {
	orders := []Order{
		{
			ID:       1001,
			Customer: Customer{Name: "Ada", Tier: Gold, Address: Address{City: "London", Country: "UK"}},
			Lines: []Line{
				{SKU: "KB-01", Qty: 1, Price: 8900},
				{SKU: "MS-02", Qty: 2, Price: 2450},
				{SKU: "CB-03", Qty: 3, Price: 990},
			},
		},
		{
			ID:       1002,
			Customer: Customer{Name: "Grace", Tier: Silver, Address: Address{City: "Arlington", Country: "US"}},
			Lines: []Line{
				{SKU: "MN-27", Qty: 1, Price: 21900},
				{SKU: "HS-01", Qty: 1, Price: 5900},
			},
		},
		{
			ID:       1003,
			Customer: Customer{Name: "Linus", Tier: Regular, Address: Address{City: "Helsinki", Country: "FI"}},
			Lines:    []Line{{SKU: "SSD-1T", Qty: 2, Price: 7450}},
		},
	}

	revenue := 0
	for _, order := range orders {
		total := orderTotal(order)
		revenue += total
		skus := make([]string, 0, len(order.Lines))
		for _, line := range order.Lines {
			skus = append(skus, line.SKU)
		}
		address := order.Customer.Address
		fmt.Printf("#%d %s (%s, %s): %s = %s\n",
			order.ID, order.Customer.Name, address.City, address.Country, strings.Join(skus, " "), money(total))
	}
	fmt.Printf("revenue %s\n", money(revenue))
}
