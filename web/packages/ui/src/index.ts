// @telemux/ui 共享包：shadcn/ui（nova 主题，radix base）基础组件 + 共享类型。
// 组件源码由 `npx shadcn@latest add ... -c packages/ui` 生成在 components/ui/。

export * from "./types"
export { cn } from "./lib/utils"

// shadcn/ui 组件（radix-nova）
export { Button, buttonVariants } from "./components/ui/button"
export {
  Card,
  CardHeader,
  CardFooter,
  CardTitle,
  CardAction,
  CardDescription,
  CardContent,
} from "./components/ui/card"
export { Input } from "./components/ui/input"
export { Label } from "./components/ui/label"
export { Badge, badgeVariants } from "./components/ui/badge"
export {
  Table,
  TableHeader,
  TableBody,
  TableFooter,
  TableHead,
  TableRow,
  TableCell,
  TableCaption,
} from "./components/ui/table"
export {
  Dialog,
  DialogTrigger,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  DialogPortal,
  DialogOverlay,
} from "./components/ui/dialog"
export {
  Select,
  SelectGroup,
  SelectValue,
  SelectTrigger,
  SelectContent,
  SelectLabel,
  SelectItem,
  SelectSeparator,
  SelectScrollUpButton,
  SelectScrollDownButton,
} from "./components/ui/select"
export { Separator } from "./components/ui/separator"
export { Switch } from "./components/ui/switch"
export { Skeleton } from "./components/ui/skeleton"
export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider } from "./components/ui/tooltip"
export { useTheme, type Theme } from "./hooks/use-theme"
