<?php

declare(strict_types=1);

namespace App\Event;

/** Calls the listeners of an event's class, as Symfony's EventDispatcher does. */
final class Dispatcher
{
    /** @var array<class-string, list<callable>> */
    private array $listeners = [];

    public function listen(string $event, callable $listener): void
    {
        $this->listeners[$event][] = $listener;
    }

    public function dispatch(object $event): void
    {
        foreach ($this->listeners[$event::class] ?? [] as $listener) {
            $listener($event);
        }
    }
}
