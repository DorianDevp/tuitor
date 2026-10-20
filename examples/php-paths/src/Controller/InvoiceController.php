<?php

declare(strict_types=1);

namespace App\Controller;

use App\Helper\Money;
use App\Repository\OrderRepository;
use App\Service\OrderService;

final class InvoiceController
{
    public function __construct(
        private readonly OrderRepository $repository,
        private readonly OrderService $orders,
    ) {
    }

    public function print(int $id): string
    {
        $order = $this->repository->find($id);
        $rows = ["invoice for order $id, {$order->customer}"];
        foreach ($order->lines as $line) {
            $rows[] = sprintf('%-6s %3d x %8s', $line->sku, $line->qty, Money::format($line->price));
        }
        $total = $this->orders->total($order);
        $rows[] = sprintf('total %s', Money::format($total));
        return implode("\n", $rows) . "\n";
    }
}
