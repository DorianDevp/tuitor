<?php

declare(strict_types=1);

namespace App\Event;

use App\Entity\Order;

final class OrderTotalled
{
    public function __construct(public readonly Order $order)
    {
    }
}
