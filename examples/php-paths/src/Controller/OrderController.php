<?php

declare(strict_types=1);

namespace App\Controller;

use App\Helper\Money;
use App\Repository\OrderRepository;
use App\Service\OrderService;

final class OrderController
{
    public function __construct(
        private readonly OrderRepository $repository,
        private readonly OrderService $orders,
    ) {
    }

    public function show(int $id): string
    {
        $order = $this->repository->find($id);
        $total = $this->orders->total($order);
        return sprintf("order %d for %s: %s\n", $order->id, $order->customer, Money::format($total));
    }
}
