export function checkout(items: number, coupon: string) {
  let fee = 0
  if (items === 0) return 0
  if (items > 10) fee += 2
  if (coupon === "VIP") fee -= 5
  if (coupon === "SAVE") fee -= 3
  if (items > 50) fee += 10
  return fee
}

export function applyCoupon(total: number, code: string) {
  if (code === "VIP") return Math.round(total * 0.85)
  if (code === "SAVE10") return Math.max(0, total - 10)
  return total
}
