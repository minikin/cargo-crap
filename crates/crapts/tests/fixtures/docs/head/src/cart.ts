export function checkout(items: number, member: boolean, coupon: string, express: boolean) {
  let fee = 0
  if (items === 0) return 0
  if (member) fee -= 5
  if (coupon !== "") fee -= 3
  if (express) fee += 10
  if (items > 10) fee += 2
  return fee
}
export function total(a: number, b: number) {
  return a + b
}
