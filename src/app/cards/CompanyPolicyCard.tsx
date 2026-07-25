import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <CompanyPolicyCard context={context} actions={actions} />
));

const CompanyPolicyCard = ({ context }: CrmExtensionProps) => {
  const recordId = context.crm.objectId;
  const portalId = context.portal.id;
  const pasUrl = `https://pas.sagesure.io/policies?hubspotPortalId=${encodeURIComponent(portalId)}&hubspotCompanyId=${encodeURIComponent(recordId)}`;

  return (
    <EmptyState
      title="SageSure-US Policy Portfolio"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Open this company&apos;s linked SagePAS policy portfolio. If no
        portfolio is linked yet, choose the correct native policy and confirm
        the link in SagePAS.
      </Text>
      <Link href={pasUrl}>Resolve company policy portfolio in SagePAS</Link>
    </EmptyState>
  );
};
