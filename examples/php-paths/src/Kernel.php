<?php

declare(strict_types=1);

namespace App;

use App\Controller\InvoiceController;
use App\Controller\OrderController;
use App\Event\Dispatcher;
use App\Event\LoyaltyListener;
use App\Event\OrderTotalled;
use App\Repository\OrderRepository;
use App\Service\OrderService;

/** Wires the services, as a container would, and routes a request to its controller. */
final class Kernel
{
    /** @var array<string, object> */
    private array $controllers;

    public function __construct()
    {
        $events = new Dispatcher();
        $events->listen(OrderTotalled::class, [new LoyaltyListener(), 'onOrderTotalled']);
        $repository = new OrderRepository();
        $orders = new OrderService($events);
        $this->controllers = [
            'order' => new OrderController($repository, $orders),
            'invoice' => new InvoiceController($repository, $orders),
        ];
    }

    /** A route is controller.action, called by name as a router does. */
    public function handle(string $route, int $id): string
    {
        [$controller, $action] = explode('.', $route);
        return $this->controllers[$controller]->$action($id);
    }
}
