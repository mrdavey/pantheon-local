import * as React from "react"
import { Popover as PopoverPrimitive } from "radix-ui"
import { cn } from "../../lib/cn"

export const Popover = PopoverPrimitive.Root
export const PopoverTrigger = PopoverPrimitive.Trigger

export function PopoverContent({ className, sideOffset = 6, ...props }: React.ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content data-slot="popover-content" sideOffset={sideOffset}
        className={cn("z-50 rounded-md border bg-background p-2 text-foreground shadow-md", className)} {...props} />
    </PopoverPrimitive.Portal>
  )
}
