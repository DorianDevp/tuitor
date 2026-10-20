<?php

declare(strict_types=1);

namespace App\Helper;

use App\Entity\Line;

final class Money
{
    /** The price of a line in cents, less 5% from ten items. */
    public static function lineTotal(Line $line): int
    {
        $amount = $line->qty * $line->price;
        if ($line->qty >= 10) {
            $amount = intdiv($amount * 95, 100);
        }
        return $amount;
    }

    public static function format(int $cents): string
    {
        return sprintf('%d.%02d', intdiv($cents, 100), $cents % 100);
    }
}
