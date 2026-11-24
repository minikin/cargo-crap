export function checkout(items: number, member: boolean) {
  let fee = 0
  if (items === 0) return 0
  if (member) fee -= 5
  return fee
}

export function applyCoupon(total: number, code: string) {
  if (code === "VIP") return Math.round(total * 0.85)
  if (code === "SAVE10") return Math.max(0, total - 10)
  return total
}
