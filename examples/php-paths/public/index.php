<?php

// The front controller: `php public/index.php order.show 1001` handles one request.

declare(strict_types=1);

// What Composer's autoloader does for composer.json's PSR-4 rule, without a composer install.
spl_autoload_register(function (string $class): void {
    $file = __DIR__ . '/../src/' . str_replace(['App\\', '\\'], ['', '/'], $class) . '.php';
    if (is_file($file)) {
        require $file;
    }
});

[, $route, $id] = $argv + [null, 'order.show', '1001'];
echo (new App\Kernel())->handle($route, (int) $id);
