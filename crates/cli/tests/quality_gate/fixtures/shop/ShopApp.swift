import SwiftUI

@main
struct ShopApp: App {
    var body: some Scene {
        WindowGroup { CartView() }
    }
}

/// One item costs $5.
let price = 5

struct CartView: View {
    @State private var items = 0

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Text("Items: \(items)")
                    .font(.title)
                    .accessibilityIdentifier("count")
                Button("Add item") { items += 1 }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("add")
                NavigationLink("Checkout") { CheckoutView(items: items) }
                    .accessibilityIdentifier("checkout")
            }
            .navigationTitle("Cart")
        }
    }
}

struct CheckoutView: View {
    let items: Int

    var total: Int {
        // The total of the items in the cart.
        max(items - 1, 0) * price
    }

    var body: some View {
        VStack(spacing: 16) {
            Text("\(items) items")
                .accessibilityIdentifier("summary")
            Text("Total: $\(total)")
                .font(.largeTitle)
                .accessibilityIdentifier("total")
        }
        .navigationTitle("Checkout")
    }
}
