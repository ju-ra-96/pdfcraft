import SwiftUI
import UniformTypeIdentifiers

/// Sign with a Digital ID: choose an ID (imported from Files, or created here), enter its
/// password and an optional reason. The signature is invisible unless the document has an
/// empty signature field, and the signed file is saved byte for byte, so it stays valid.
struct SignSheet: View {
    var onSign: (String, String, String) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var ids = DigitalIDs.list()
    @State private var selected: String?
    @State private var password = ""
    @State private var reason = ""
    @State private var importing = false
    @State private var creating = false
    @State private var newName = ""
    @State private var newEmail = ""
    @State private var newPassword = ""
    @State private var problem: String?

    var body: some View {
        NavigationStack {
            Form {
                Section(String(localized: "Digital ID")) {
                    if ids.isEmpty {
                        Text("No digital IDs yet. Import a .p12 or .pfx file, or create one.").foregroundStyle(.secondary)
                    }
                    ForEach(ids, id: \.self) { id in
                        Button {
                            selected = id
                        } label: {
                            HStack {
                                Label(id, systemImage: "person.text.rectangle")
                                Spacer()
                                if selected == id { Image(systemName: "checkmark").foregroundStyle(Color.accentColor) }
                            }
                        }
                        .foregroundStyle(.primary)
                    }
                    Button { importing = true } label: { Label(String(localized: "Import a Digital ID…"), systemImage: "square.and.arrow.down") }
                    Button { creating = true } label: { Label(String(localized: "Create a Digital ID…"), systemImage: "plus.circle") }
                        .accessibilityIdentifier("create-id")
                }
                if selected != nil {
                    Section(String(localized: "Signature")) {
                        SecureField(String(localized: "Password of the digital ID"), text: $password)
                            .accessibilityIdentifier("id-password")
                        TextField(String(localized: "Reason (optional)"), text: $reason)
                    }
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(.red) }
                }
            }
            .navigationTitle(String(localized: "Sign with a Digital ID"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(String(localized: "Cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Sign")) {
                        guard let selected else { return }
                        onSign(selected, password, reason)
                        dismiss()
                    }
                    .disabled(selected == nil || password.isEmpty)
                    .accessibilityIdentifier("sign")
                }
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.digitalID]) { result in
                switch result {
                case .success(let url):
                    do {
                        let name = try DigitalIDs.importFile(url)
                        ids = DigitalIDs.list()
                        selected = name
                    } catch {
                        problem = error.localizedDescription
                    }
                case .failure(let error): problem = error.localizedDescription
                }
            }
            .sheet(isPresented: $creating) { createForm }
        }
    }

    private var createForm: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(String(localized: "Name"), text: $newName).accessibilityIdentifier("id-name")
                    TextField(String(localized: "Email (optional)"), text: $newEmail).keyboardType(.emailAddress).textInputAutocapitalization(.never)
                    SecureField(String(localized: "Password (at least 6 characters)"), text: $newPassword).accessibilityIdentifier("id-new-password")
                } footer: {
                    Text("A self-signed ID: others see your name, but must trust the certificate before the signature shows as valid.")
                }
            }
            .navigationTitle(String(localized: "Create a Digital ID"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(String(localized: "Cancel")) { creating = false } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Create")) { create() }
                        .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty || newPassword.count < 6)
                        .accessibilityIdentifier("create")
                }
            }
        }
    }

    private func create() {
        do {
            let name = try DigitalIDs.create(name: newName.trimmingCharacters(in: .whitespaces), email: newEmail, password: newPassword)
            ids = DigitalIDs.list()
            selected = name
            password = newPassword
            creating = false
        } catch {
            problem = error.localizedDescription
            creating = false
        }
    }
}
