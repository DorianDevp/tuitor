<?php

declare(strict_types=1);

namespace App\Service;

use App\Entity\Order;
use App\Event\Dispatcher;
use App\Event\OrderTotalled;
use App\Helper\Money;

final class OrderService
{
    public function __construct(private readonly Dispatcher $events)
    {
    }

    /** The order's total in cents, less 10% for gold customers. */
    public function total(Order $order): int
    {
        $total = 0;
        foreach ($order->lines as $line) {
            $total += Money::lineTotal($line);
        }
        if ($order->gold) {
            $total -= intdiv($total, 10);
        }
        $order->total = $total;
        $this->events->dispatch(new OrderTotalled($order));
        return $total;
    }
}
